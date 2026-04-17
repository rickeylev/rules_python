import argparse
import sys
import zipfile

def create_zip(zip_path, files):
    files.sort(key=lambda x: x[0])
    with zipfile.ZipFile(zip_path, 'w', compression=zipfile.ZIP_STORED) as zf:
        for name, path in files:
            zinfo = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            with open(path, 'rb') as f_in:
                zf.writestr(zinfo, f_in.read())

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
            
            zip_path, content_path = line.split('|', 1)
            
            if hasattr(zip_path, "removeprefix"):
                zip_path = zip_path.removeprefix(prefix)
            else:
                if zip_path.startswith(prefix):
                    zip_path = zip_path[len(prefix):]
                    
            entries.append((zip_path, content_path))
        
    create_zip(args.out, entries)

if __name__ == "__main__":
    main()
