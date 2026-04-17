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
            file_path = line.strip()
            if not file_path:
                continue
            name = file_path
            if file_path.startswith(prefix):
                name = file_path[len(prefix):]
            entries.append(FileEntry(file_path, name))
        
    create_deterministic_zip(args.out, entries)

if __name__ == "__main__":
    main()
