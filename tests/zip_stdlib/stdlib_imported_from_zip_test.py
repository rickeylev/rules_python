import unittest
import pathlib
import os
import sys
import json
import logging
import shutil
import tarfile
import urllib.request
import concurrent.futures
import sqlite3
import re
import math
import collections

class StdlibImportedFromZipTest(unittest.TestCase):
    def test_imports_work(self):
        self.assertTrue(hasattr(pathlib, 'Path'))
        self.assertTrue(hasattr(os, 'path'))
        
        modules_to_check = [
            pathlib,
            os,
            json,
            logging,
            shutil,
            tarfile,
            urllib.request,
            concurrent.futures,
            sqlite3,
            re,
            collections,
        ]
        
        for mod in modules_to_check:
            # Some modules might be built-ins and don't have __file__, or are native extensions
            # But the ones we picked (except maybe math/re which could be extensions, let's pick pure python ones)
            if hasattr(mod, "__file__"):
                mod_file = mod.__file__
                # Ignore dynamic extensions (.so/.pyd) which cannot be loaded from zip directly
                if not mod_file.endswith(".so") and not mod_file.endswith(".pyd"):
                    self.assertIn(".zip", mod_file, f"{mod.__name__} was not loaded from a zip file: {mod_file}")

if __name__ == "__main__":
    unittest.main()
