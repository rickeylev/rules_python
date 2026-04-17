import os
import sys

def main():
    runfiles_dir = os.environ.get("RUNFILES_DIR", ".")
    found_zip = False
    found_os_py = False

    for root, dirs, files in os.walk(runfiles_dir):
        for file in files:
            print("RUNFILE:", os.path.join(root, file))
            if file.endswith(".zip"):
                print("FOUND ZIP:", root, file)
                
        if "python3.9.zip" in files:
            found_zip = True
            
        if "os.py" in files and "python3.9" in root:
            if "tests/py_runtime/lib/python3.9" in root.replace(os.sep, "/"):
                found_os_py = True

    if not found_zip:
        print("FAIL: python3.9.zip not found")
        sys.exit(1)
        
    if found_os_py:
        print("FAIL: os.py should not be present in runfiles")
        sys.exit(1)
    
    print("PASS: zip file found and original files omitted from runfiles.")

if __name__ == "__main__":
    main()
