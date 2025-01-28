
import sys
import os

for k, v in os.environ.items():
    print(k, v)

print(sys.version)

m = sys.modules["STASH_COVERAGE"]
print(m)
raise Exception('hit')
