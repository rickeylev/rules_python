import argparse
import sys
import zipfile

def create_deterministic_zip(zip_path, files):
    files.sort()
    with zipfile.ZipFile(zip_path, 'w', compression=zipfile.ZIP_STORED) as zf:
        for f in files:
            zinfo = zipfile.ZipInfo(f.name, date_time=(1980, 1, 1, 0, 0, 0))
            with open(f.path, 'rb') as f_in:
                zf.writestr(zinfo, f_in.read())

class FileEntry:
    def __init__(self, path, name):
        self.path = path
        self.name = name

    def __lt__(self, other):
        return self.name < other.name

def main(argv=None):
    parser = argparse.ArgumentParser(description="Deterministic zip for stdlib")
    parser.add_argument("--out", required=True, help="Output zip file path")
    parser.add_argument("--strip-prefix", required=True, help="Prefix to strip from paths")
    parser.add_argument("--manifest", required=True, help="Path to manifest file containing files to zip")
    
    args = parser.parse_args(argv)
    
    prefix = args.strip_prefix
    if not prefix.endswith("/"):
        prefix += "/"
        
    entries = []
    with open(args.manifest, 'r') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            parts = line.split('|', 2)
            if len(parts) == 3:
                entry_type, zip_path, content_path = parts
                if entry_type == 'f' or entry_type == 'file':
                    if zip_path.startswith(prefix):
                        zip_path = zip_path[len(prefix):]
                    entries.append(FileEntry(content_path, zip_path))
            else:
                print("Invalid manifest line: " + line)
                sys.exit(1)
        
    create_deterministic_zip(args.out, entries)

if __name__ == "__main__":
    main()
