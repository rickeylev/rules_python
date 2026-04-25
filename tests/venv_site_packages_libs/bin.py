import importlib
import os
import sys
import unittest
from pathlib import Path


class VenvSitePackagesLibraryTest(unittest.TestCase):
    def setUp(self):
        super().setUp()
        if sys.prefix == sys.base_prefix:
            raise AssertionError("Not running under a venv")
        self.venv = sys.prefix

    def assert_imported_from_venv(self, module_name):
        module = importlib.import_module(module_name)
        self.assertEqual(module.__name__, module_name)
        self.assertIsNotNone(
            module.__file__,
            f"Expected module {module_name!r} to have"
            + f"__file__ set, but got None. {module=}",
        )
        self.assertTrue(
            module.__file__.startswith(self.venv),
            f"\n{module_name} was imported, but not from the venv.\n"
            + f"       venv: {self.venv}\n"
            + f"module file: {module.__file__}\n"
            + "sys.path:\n"
            + "\n".join(sys.path),
        )
        return module

    def test_imported_from_venv(self):
        m = self.assert_imported_from_venv("pkgutil_top")
        self.assertEqual(m.WHOAMI, "pkgutil_top")

        m = self.assert_imported_from_venv("pkgutil_top.sub")
        self.assertEqual(m.WHOAMI, "pkgutil_top.sub")

        self.assert_imported_from_venv("nspkg.subnspkg.alpha")
        self.assert_imported_from_venv("nspkg.subnspkg.beta")
        self.assert_imported_from_venv("nspkg.subnspkg.gamma")
        self.assert_imported_from_venv("nspkg.subnspkg.delta")
        self.assert_imported_from_venv("single_file")
        self.assert_imported_from_venv("simple")
        m = self.assert_imported_from_venv("nested_with_pth")
        self.assertEqual(m.WHOAMI, "nested_with_pth")

    def test_data_is_included(self):
        self.assert_imported_from_venv("simple")
        module = importlib.import_module("simple")
        module_path = Path(module.__file__)

        site_packages = module_path.parent.parent

        # Ensure that packages from simple v1 are not present
        files = [p.name for p in site_packages.glob("*")]
        self.assertIn("simple_v1_extras", files)

    def test_override_pkg(self):
        self.assert_imported_from_venv("simple")
        module = importlib.import_module("simple")
        self.assertEqual(
            "1.0.0",
            module.__version__,
        )

    def test_dirs_from_replaced_package_are_not_present(self):
        self.assert_imported_from_venv("simple")
        module = importlib.import_module("simple")
        module_path = Path(module.__file__)

        site_packages = module_path.parent.parent
        dist_info_dirs = [p.name for p in site_packages.glob("simple*.dist-info")]
        self.assertEqual(
            ["simple-1.0.0.dist-info"],
            dist_info_dirs,
        )

        # Ensure that packages from simple v1 are not present
        files = [p.name for p in site_packages.glob("*")]
        self.assertNotIn("simple.libs", files)

    def test_data_from_another_pkg_is_included_via_copy_file(self):
        self.assert_imported_from_venv("simple")
        module = importlib.import_module("simple")
        module_path = Path(module.__file__)

        site_packages = module_path.parent.parent
        # Ensure that packages from simple v1 are not present
        d = site_packages / "external_data"
        files = [p.name for p in d.glob("*")]
        self.assertIn("another_module_data.txt", files)

    @unittest.skipIf(
        os.environ.get("BZLMOD_ENABLED") == "0",
        "whl_with_data is only available with bzlmod",
    )
    def test_whl_with_data_included(self):
        module = self.assert_imported_from_venv("whl_with_data")
        module_path = Path(module.__file__)
        site_packages = module_path.parent.parent

        # purelib
        data_file = site_packages / "whl_with_data" / "data_file.txt"
        self.assertTrue(data_file.exists(), f"Expected {data_file} to exist")

        # platlib
        platlib_file = site_packages / "whl_with_data" / "platlib_file.txt"
        self.assertTrue(platlib_file.exists(), f"Expected {platlib_file} to exist")

        venv_root = Path(self.venv)

        is_windows = sys.platform == "win32"
        if is_windows:
            bin_dir_name = "Scripts"
            include_dir_name = "Include"
        else:
            bin_dir_name = "bin"
            include_dir_name = "include"

        # data
        data_data_file = venv_root / "data" / "whl_with_data" / "data_data_file.txt"
        self.assertTrue(
            data_data_file.exists(),
            f"Expected {data_data_file} to exist. venv_root contents: {list(venv_root.iterdir()) if venv_root.exists() else 'N/A'}. os.name={os.name}, sys.platform={sys.platform}",
        )

        # scripts
        script_file = venv_root / bin_dir_name / "whl_script.sh"
        self.assertTrue(
            script_file.exists(),
            f"Expected {script_file} to exist. {bin_dir_name} contents: {list((venv_root / bin_dir_name).iterdir()) if (venv_root / bin_dir_name).exists() else 'N/A'}",
        )

        # headers
        header_file = venv_root / include_dir_name / "whl_with_data" / "header_file.h"
        self.assertTrue(
            header_file.exists(),
            f"Expected {header_file} to exist. {include_dir_name} contents: {list((venv_root / include_dir_name).iterdir()) if (venv_root / include_dir_name).exists() else 'N/A'}",
        )


if __name__ == "__main__":
    unittest.main()
