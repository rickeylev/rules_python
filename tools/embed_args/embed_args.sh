#!/bin/sh
# embed_args.sh
#
# Embeds arbitrary arguments into a binary executable by appending a trailer
# payload. This allows creating pre-configured executable wrappers (such as
# launcher scripts or tool wrappers) without requiring compilation toolchains
# or shell script wrappers.
#
# Binary Trailer Layout:
# ----------------------
# At the very end of the output binary, the file is laid out as follows:
#
#   +-------------------------------------------------------------+
#   | Original Executable Bytes                                   |
#   +-------------------------------------------------------------+
#   | Payload Bytes:                                              |
#   |   arg1\0arg2\0...argN\0                                     |
#   +-------------------------------------------------------------+
#   | Payload Length (8 bytes, little-endian u64)                 |
#   +-------------------------------------------------------------+
#   | Magic Identifier (24 bytes):                                |
#   |   "!ARGS\0\0\0" (8 bytes ASCII + zero padding)              |
#   |   UUID 6ca2171c-7b91-4fe8-87b3-ce5912466b47 (16 raw bytes   |
#   |   in big-endian / RFC 4122 network byte order:              |
#   |   0x6c 0xa2 0x17 0x1c 0x7b 0x91 0x4f 0xe8                   |
#   |   0x87 0xb3 0xce 0x59 0x12 0x46 0x6b 0x47)                  |
#   +-------------------------------------------------------------+
#
# Total footer size after payload = 8 + 24 = 32 bytes (8-byte aligned).
#
# Payload Encoding:
# -----------------
# The payload consists of zero or more null-terminated byte strings (`\0`).
# By convention, key-value configuration is formatted as `key=value\0`
# (e.g., `src_out=path/to/src\0out=path/to/out\0arg=--flag\0`).
#
# Deserialization Algorithm (for executable binaries reading the trailer):
# ------------------------------------------------------------------------
# 1. Open the current executable binary file for reading.
# 2. Query file size (`len`). If `len < 32`, trailer is not present.
# 3. Seek to `len - 32` and read 32 bytes:
#    - Bytes [0..8]: `payload_len` as little-endian 64-bit integer (`u64`).
#    - Bytes [8..32]: Magic identifier (must match `!ARGS\0\0\0` + 16-byte UUID).
# 4. If magic does not match or `len < 32 + payload_len`, trailer is invalid.
# 5. Seek to `len - 32 - payload_len` and read `payload_len` bytes.
# 6. Split the read bytes on `\0` delimiters to recover individual arguments.
# 7. For `key=value` entries, split each entry at the first `=` byte.
#
set -eu

if [ $# -lt 2 ]; then
    echo "Usage: $0 <input_file> <output_file> [ARGS...]" >&2
    exit 1
fi

INPUT="$1"
OUTPUT="$2"
shift 2

PAYLOAD_TMP=$(mktemp)
trap 'rm -f "$PAYLOAD_TMP"' EXIT

while [ $# -gt 0 ]; do
    printf '%s\0' "$1" >> "$PAYLOAD_TMP"
    shift
done

PAYLOAD_LEN=$(LC_ALL=C wc -c < "$PAYLOAD_TMP" | tr -d ' \t\n\r')

OUT_DIR=$(dirname "$OUTPUT")
if [ -n "$OUT_DIR" ] && [ ! -d "$OUT_DIR" ]; then
    mkdir -p "$OUT_DIR"
fi

cp -f "$INPUT" "$OUTPUT"
chmod +w "$OUTPUT" 2>/dev/null || true

if [ "$PAYLOAD_LEN" -gt 0 ]; then
    cat "$PAYLOAD_TMP" >> "$OUTPUT"
fi

b0=$(( PAYLOAD_LEN & 255 ))
b1=$(( (PAYLOAD_LEN >> 8) & 255 ))
b2=$(( (PAYLOAD_LEN >> 16) & 255 ))
b3=$(( (PAYLOAD_LEN >> 24) & 255 ))
b4=$(( (PAYLOAD_LEN >> 32) & 255 ))
b5=$(( (PAYLOAD_LEN >> 40) & 255 ))
b6=$(( (PAYLOAD_LEN >> 48) & 255 ))
b7=$(( (PAYLOAD_LEN >> 56) & 255 ))

o0=$(printf '%03o' "$b0")
o1=$(printf '%03o' "$b1")
o2=$(printf '%03o' "$b2")
o3=$(printf '%03o' "$b3")
o4=$(printf '%03o' "$b4")
o5=$(printf '%03o' "$b5")
o6=$(printf '%03o' "$b6")
o7=$(printf '%03o' "$b7")

# 24-byte magic: "!ARGS\0\0\0" (8 bytes) + 16-byte raw binary UUID: 6ca2171c-7b91-4fe8-87b3-ce5912466b47
# Fixed footer = 8 bytes length + 24 bytes magic = 32 bytes total.
MAGIC_UUID='\154\242\027\034\173\221\117\350\207\263\316\131\022\106\153\107'

printf "\\${o0}\\${o1}\\${o2}\\${o3}\\${o4}\\${o5}\\${o6}\\${o7}!ARGS\\000\\000\\000${MAGIC_UUID}" >> "$OUTPUT"

chmod +x "$OUTPUT"
