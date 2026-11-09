"""Transfer only an APK v2/v3 signature for a verified Actions artifact."""

import argparse
import base64
import hashlib
import json
import shutil
import struct
import subprocess
from pathlib import Path

CERT_SHA256 = "bce0bb760a1076dd7ba6d8502b78bb094b0fac6ab83e97d3448a7c7ec5242d88"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def central_directory(path):
    with path.open("rb") as stream:
        stream.seek(max(0, path.stat().st_size - 65557))
        start = stream.tell()
        tail = stream.read()
    index = tail.rfind(b"PK\x05\x06")
    if index < 0 or index + 22 != len(tail):
        raise ValueError("Expected an APK with a comment-free ZIP end record")
    directory = struct.unpack_from("<I", tail, index + 16)[0]
    return directory, start + index


def copy_part(source, target, length):
    while length:
        chunk = source.read(min(length, 1024 * 1024))
        if not chunk:
            raise ValueError("APK ended before its ZIP end record")
        target.write(chunk)
        length -= len(chunk)


def apply(unsigned, receipt, output):
    if digest(unsigned) != receipt["unsigned_sha256"]:
        raise ValueError("Unsigned APK differs from the inspected build")
    block = base64.b64decode(receipt["block"], validate=True)
    if len(block) > 256 * 1024 or not block.endswith(b"APK Sig Block 42"):
        raise ValueError("Invalid APK signature block")
    directory, end = central_directory(unsigned)
    with unsigned.open("rb") as source, output.open("wb") as target:
        copy_part(source, target, directory)
        target.write(block)
        copy_part(source, target, end + 16 - directory)
        source.read(4)
        target.write(struct.pack("<I", directory + len(block)))
        shutil.copyfileobj(source, target)
    if digest(output) != receipt["signed_sha256"]:
        raise ValueError("Reassembled APK does not match the signed APK")


def verify(signer, apk):
    result = subprocess.check_output([signer, "verify", "--verbose", "--print-certs", apk], text=True)
    expected = f"Signer #1 certificate SHA-256 digest: {CERT_SHA256}"
    if expected not in result.splitlines() or "Number of signers: 1" not in result.splitlines():
        raise ValueError("APK does not have the pinned Thor test signer")
    print(result)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--apksigner", required=True)
commands = parser.add_subparsers(dest="mode", required=True)
create = commands.add_parser("record")
create.add_argument("unsigned", type=Path)
create.add_argument("signed", type=Path)
create.add_argument("run_id", type=int)
create.add_argument("source_sha")
create.add_argument("receipt", type=Path)
restore = commands.add_parser("apply")
restore.add_argument("unsigned", type=Path)
restore.add_argument("receipt", type=Path)
restore.add_argument("signed", type=Path)
args = parser.parse_args()
if args.mode == "record":
    import tempfile

    verify(args.apksigner, args.signed)
    directory, _ = central_directory(args.unsigned)
    signed_directory, _ = central_directory(args.signed)
    length = signed_directory - directory
    if not 0 < length <= 256 * 1024:
        raise ValueError("APK signer rewrote ZIP contents or wrote an oversized block")
    with args.signed.open("rb") as stream:
        stream.seek(directory)
        block = stream.read(length)
    receipt = {
        "run_id": args.run_id,
        "source_sha": args.source_sha,
        "unsigned_sha256": digest(args.unsigned),
        "signed_sha256": digest(args.signed),
        "block": base64.b64encode(block).decode("ascii"),
    }
    with tempfile.TemporaryDirectory() as tmp:
        apply(args.unsigned, receipt, Path(tmp) / "verified.apk")
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
else:
    apply(args.unsigned, json.loads(args.receipt.read_text()), args.signed)
    verify(args.apksigner, args.signed)
