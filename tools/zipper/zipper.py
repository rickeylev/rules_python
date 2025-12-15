import argparse
import os
import shutil
import stat
import sys
import zipfile

# Unix permission bit for symlink (S_IFLNK)
# S_IFLNK is usually 0o120000
S_IFLNK = 0o120000


def _add_entry(zf, line, line_idx, compress_type):
    line = line.strip()
    if not line:
        return

    parts = line.split("|")
    if len(parts) != 3:
        raise ValueError(f"Error: Invalid line format at line {line_idx + 1}: {line}")

    is_symlink_str, zip_path, content_path = parts
    is_symlink = is_symlink_str == "1"

    zi = zipfile.ZipInfo(zip_path)
    zi.create_system = 3  # Unix
    zi.compress_type = compress_type
    if is_symlink:
        target = os.readlink(content_path)
        # Set permissions to 777 for symlink (standard)
        zi.external_attr = (S_IFLNK | 0o777) << 16
        zf.writestr(zi, target)
    else:
        st = os.stat(content_path)
        # Preserve permissions, otherwise execute is dropped.
        zi.external_attr = (st.st_mode & 0xFFFF) << 16
        with open(content_path, "rb") as src, zf.open(zi, "w") as dst:
            shutil.copyfileobj(src, dst)


def create_zip(manifest_path, output_zip, compress_level=0):
    compress_type = zipfile.ZIP_STORED if compress_level == 0 else zipfile.ZIP_DEFLATED
    zf_level = compress_level if compress_level != 0 else None

    with zipfile.ZipFile(
        output_zip, "w", compress_type, allowZip64=True, compresslevel=zf_level
    ) as zf:
        with open(manifest_path, "r") as f:
            # Sort for determinism.
            entries = sorted(enumerate(f), key=lambda x: x[1])
            for line_idx, line in entries:
                try:
                    _add_entry(zf, line, line_idx, compress_type)
                except OSError as e:
                    e.add_note(f"Error processing line {line_idx + 1}: {line.strip()}")
                    raise


def main():
    parser = argparse.ArgumentParser(description="Create a zip file from a manifest.")
    parser.add_argument(
        "manifest",
        help="""
Path to the manifest file. Each line has format `is_symlink|zip_path|content_path`
`is_symlink` is "0" or "1" to denote if the file should be stored as a symlink.
`zip_path` is the path within the zipfile to store. `content_path` is the
content to store at `zip_path`, or, if its a symlink, the symlink whose value
to store at `zip_path`. Empty lines are ignored.
""",
    )
    parser.add_argument("output", help="Path to the output zip file.")
    parser.add_argument(
        "--compression",
        type=int,
        default=0,
        help="Compression level (0 for stored, others for deflated)",
    )
    args = parser.parse_args()

    try:
        create_zip(args.manifest, args.output, compress_level=args.compression)
    except Exception as e:
        e.add_note(f"Error creating zip {args.output}")
        raise


if __name__ == "__main__":
    main()
