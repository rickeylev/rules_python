# Copyright 2024 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import unittest
import sys
import types
import importlib
import importlib.util
import importlib.machinery
import os.path

# This is a simplified version of a hack ChromeOS does in their toolchain's
# sitecustomize.py module. It basically pre-populates fake modules for
# repos in order to make `import reponame` work with bzlmod.
# Unfortunately, this has the side effect of causing the rules_python repo-root
# level __init__.py to be ignored, which means the module patching it does
# is skipped.
# See https://source.chromium.org/chromiumos/chromiumos/codesearch/+/main:src/bazel/python/toolchains/sitecustomize.py;drc=b9ec11186e43095c44b5f4da533eb84302df77f6
class FakeRulesPython(types.ModuleType):
    def __init__(self, *, repo_root):
        super().__init__(name="rules_python")
        self.__file__ = f"fake @rules_python module"
        self._repo_root = repo_root

        spec = importlib.machinery.ModuleSpec(
            name=self.__name__,
            loader=None,
        )
        spec.submodule_search_locations = [repo_root]
        mod = importlib.util.module_from_spec(spec)
        self._mod = mod

    def __getattr__(self, item):
        """Dispatches the getattr to the real module."""
        return getattr(self._mod, item)


class ImportFakedRulesPythonTest(unittest.TestCase):
    def test_import_rules_python(self):
        import rules_python as real_rules_python
        from rules_python.python.runfiles import runfiles as real_runfiles
        rf = real_runfiles.Create()
        runfiles_root = rf._python_runfiles_root

        # With bzlmod, the ('', 'rules_python') entry is present (maps
        # the current repo's concept of "rules_python" to its runfiles name.
        # Without bzlmod, the entry isn't present, so no mapping is needed,
        # and we can just use plain rules_python.
        rules_python_dirname = rf._repo_mapping.get(('', 'rules_python'), 'rules_python')
        repo_root = os.path.join(runfiles_root, rules_python_dirname)

        # Clear out all the imports from using the runfiles. We don't want
        # them to interfere with the real imports
        for name in list(sys.modules.keys()):
            if name == "rules_python" or name.startswith("rules_python."):
                del sys.modules[name]
        importlib.invalidate_caches()

        fake_rules_python = FakeRulesPython(repo_root=str(repo_root))
        sys.modules["rules_python"] = fake_rules_python

        import rules_python

        # If these were the same, then the point of the test would be defeated.
        self.assertIsNot(rules_python, real_rules_python)

        import rules_python.python
        import rules_python.python.runfiles
        import rules_python.python.runfiles.runfiles

        import python
        import python.runfiles
        import python.runfiles.runfiles

        self.assertIs(rules_python.python, python)
        self.assertIs(rules_python.python.runfiles, python.runfiles)
        self.assertIs(rules_python.python.runfiles.runfiles, python.runfiles.runfiles)


if __name__ == "__main__":
    unittest.main()
