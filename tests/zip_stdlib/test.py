import sys
import os

runfiles_dir = os.environ.get("RUNFILES_DIR", os.getcwd())
for root, dirs, files in os.walk(runfiles_dir):
    for file in files:
        if file.startswith("python") and file.endswith(".zip") and "lib" in root:
            zip_path = os.path.join(root, file)
            sys.path.insert(0, zip_path)
            break

import pathlib

def main():
    pathlib_file = pathlib.__file__
    print(f"pathlib is loaded from: {pathlib_file}")
    
    if ".zip" not in pathlib_file:
        print("FAIL: pathlib was not loaded from a zip file.")
        sys.exit(1)
    
    print("PASS: pathlib is loaded from a zip file.")

if __name__ == "__main__":
    main()
