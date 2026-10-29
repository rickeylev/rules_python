"""Convert a parsed uv.lock to requirements.txt format."""

def uv_lock_extras_map(uv_lock):
    """Compute extras for each package from uv.lock data.

    Args:
      uv_lock: a decoded JSON struct from a uv.lock file.

    Returns:
      A dict of {package_name: [extra1, extra2, ...]} for packages with extras.
    """
    extras_map = {}
    for pkg in uv_lock.get("package", []):
        pkg_name = pkg.get("name", "")

        for extra in pkg.get("provides-extras", pkg.get("extras", [])):
            _add_extras(extras_map, pkg_name, [extra])

        opt_deps = pkg.get("optional-dependencies", {})
        if opt_deps:
            _add_extras(extras_map, pkg_name, _sorted(opt_deps.keys()))

        deps = pkg.get("dependencies", [])
        for dep in deps:
            dep_name = dep.get("name", "")
            dep_extras_raw = dep.get("extra", [])
            dep_extras = [dep_extras_raw] if type(dep_extras_raw) == "string" else dep_extras_raw
            if dep_extras and dep_name != pkg_name:
                _add_extras(extras_map, dep_name, dep_extras)

        metadata = pkg.get("metadata", {})
        for rd in metadata.get("requires-dist", []):
            rd_name = rd.get("name", "")
            rd_extras_raw = rd.get("extras", [])
            rd_extras = [rd_extras_raw] if type(rd_extras_raw) == "string" else rd_extras_raw
            if rd_extras and rd_name != pkg_name:
                _add_extras(extras_map, rd_name, rd_extras)

    return extras_map

def _uv_lock_markers_map(uv_lock):
    """Compute effective environment markers for each package in uv.lock.

    Propagates dependency-edge markers from root packages through the
    dependency graph and combines them with package-level resolution-markers.

    Args:
      uv_lock: a decoded JSON struct from a uv.lock file.

    Returns:
      A dict mapping (package_name, version) to a PEP 508 marker string ("" if
      unconditional).
    """
    packages = uv_lock.get("package", [])
    if not packages:
        return {}

    by_name = {}
    by_name_version = {}
    for idx, pkg in enumerate(packages):
        pkg_name = pkg.get("name", "")
        pkg_version = pkg.get("version", "")
        by_name.setdefault(pkg_name, []).append(idx)
        by_name_version[(pkg_name, pkg_version)] = idx

    outgoing = [[] for _ in range(len(packages))]
    has_incoming = [False for _ in range(len(packages))]
    for idx, pkg in enumerate(packages):
        pkg_name = pkg.get("name", "")
        all_deps = list(pkg.get("dependencies", []))
        for _extra, deps in pkg.get("optional-dependencies", {}).items():
            all_deps.extend(deps)
        for _group, deps in pkg.get("dev-dependencies", {}).items():
            all_deps.extend(deps)

        for dep in all_deps:
            dep_name = dep.get("name", "")
            if not dep_name or dep_name == pkg_name:
                continue
            dep_version = dep.get("version", "")
            if dep_version and (dep_name, dep_version) in by_name_version:
                targets = [by_name_version[(dep_name, dep_version)]]
            else:
                targets = by_name.get(dep_name, [])
            edge_marker = dep.get("marker", "")
            for target_idx in targets:
                has_incoming[target_idx] = True
                outgoing[idx].append((target_idx, edge_marker))

    # reach[idx] is True (unconditionally reachable from a root) or a list of
    # disjunctive path marker strings.
    reach = []
    for idx, pkg in enumerate(packages):
        source = pkg.get("source", {})
        is_root = (
            not has_incoming[idx] or
            bool(source.get("virtual")) or
            bool(source.get("editable"))
        )
        reach.append(True if is_root else [])

    for _ in range(len(packages)):
        changed = False
        for u in range(len(packages)):
            u_reach = reach[u]
            if u_reach == []:
                continue
            for v, edge_marker in outgoing[u]:
                if reach[v] == True:
                    continue
                if u_reach == True:
                    if not edge_marker:
                        reach[v] = True
                        changed = True
                    elif edge_marker not in reach[v]:
                        reach[v].append(edge_marker)
                        changed = True
                else:
                    for d in u_reach:
                        combined = _and_combine(d, edge_marker) if edge_marker else d
                        if combined not in reach[v]:
                            reach[v].append(combined)
                            changed = True
        if not changed:
            break

    markers_map = {}
    for idx, pkg in enumerate(packages):
        pkg_name = pkg.get("name", "")
        pkg_version = pkg.get("version", "")
        v_reach = reach[idx]
        if v_reach == True or v_reach == []:
            reach_expr = ""
        else:
            reach_expr = " or ".join(v_reach)

        res_markers = pkg.get("resolution-markers", [])
        res_expr = " or ".join(res_markers) if res_markers else ""

        if reach_expr and res_expr:
            final_expr = _and_combine(reach_expr, res_expr)
        else:
            final_expr = reach_expr or res_expr

        markers_map[(pkg_name, pkg_version)] = final_expr

    return markers_map

def uv_lock_to_requirements(uv_lock):
    """Convert a parsed uv.lock JSON struct to a requirements.txt formatted string.

    Args:
      uv_lock: a decoded JSON struct from a uv.lock file.

    Returns:
      A requirements.txt formatted string.
    """
    packages = uv_lock.get("package", [])
    extras_map = uv_lock_extras_map(uv_lock)
    markers_map = _uv_lock_markers_map(uv_lock)

    dependents = {}
    for pkg in packages:
        pkg_name = pkg.get("name", "")
        deps = pkg.get("dependencies", [])
        for dep in deps:
            dep_name = dep.get("name", "")
            if dep_name != pkg_name:
                _add_dependent(dependents, dep_name, pkg_name)
        opt_deps = pkg.get("optional-dependencies", {})
        for _extra, deps in opt_deps.items():
            for dep in deps:
                dep_name = dep.get("name", "")
                if dep_name != pkg_name:
                    _add_dependent(dependents, dep_name, pkg_name)

    lines = []
    for pkg in packages:
        source = pkg.get("source", {})
        if not source.get("registry"):
            continue

        pkg_name = pkg.get("name", "")
        version = pkg.get("version", "")

        marker_expr = markers_map.get((pkg_name, version), "")
        hashes = _collect_hashes(pkg)

        pkg_extras = extras_map.get(pkg_name, [])
        if pkg_extras:
            req = "{}[{}]=={}".format(pkg_name, ",".join(pkg_extras), version)
        else:
            req = "{}=={}".format(pkg_name, version)
        if marker_expr:
            req += " ; " + marker_expr

        if hashes:
            req += " \\"
        lines.append(req)

        _emit_hashes(lines, hashes)

        dep_vias = _sorted(dependents.get(pkg_name, []))
        if dep_vias:
            if len(dep_vias) == 1:
                lines.append("    # via " + dep_vias[0])
            else:
                lines.append("    # via")
                for via in dep_vias:
                    lines.append("    #   " + via)

        lines.append("")

    return "\n".join(lines)

def _and_combine(a, b):
    if not a:
        return b
    if not b or a == b:
        return a
    a_clauses = a.split(" and ") if " or " not in a else [a]
    b_clauses = b.split(" and ") if " or " not in b else [b]
    if all([c in a_clauses for c in b_clauses]):
        return a
    if all([c in b_clauses for c in a_clauses]):
        return b
    a_str = "({})".format(a) if " or " in a else a
    b_str = "({})".format(b) if " or " in b else b
    return "{} and {}".format(a_str, b_str)

def _add_dependent(dependents, dep_name, dependent_name):
    if dep_name not in dependents:
        dependents[dep_name] = []
    if dependent_name not in dependents[dep_name]:
        dependents[dep_name].append(dependent_name)

def _add_extras(extras_map, pkg_name, extras):
    existing = extras_map.get(pkg_name, [])
    for extra in extras:
        if extra not in existing:
            existing.append(extra)
    extras_map[pkg_name] = _sorted(existing)

def _collect_hashes(pkg):
    hashes = []
    for wheel in pkg.get("wheels", []):
        whash = wheel.get("hash", "")
        if whash.startswith("sha256:"):
            hashes.append(whash[len("sha256:"):])
    sdist = pkg.get("sdist")
    if sdist:
        shash = sdist.get("hash", "")
        if shash.startswith("sha256:"):
            hashes.append(shash[len("sha256:"):])
    return _sorted(hashes)

def _sorted(items):
    return sorted(items)

def _emit_hashes(lines, hashes):
    for i, h in enumerate(hashes):
        suffix = " \\" if i < len(hashes) - 1 else ""
        lines.append("    --hash=sha256:{}{}".format(h, suffix))
