#!/usr/bin/env python3
"""Package a built application beside its verified offline runtime resources."""
import argparse
from pathlib import Path
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target', required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--profile', choices=['debug', 'release'], default='release')
parser.add_argument('--binary', type=Path, help='Explicit built executable, e.g. a stripped copy')
args = parser.parse_args()
name = 'codestyles.exe' if 'windows' in args.target else 'codestyles'
binary = args.binary or ROOT / 'target' / args.profile / name
if not binary.is_file():
    parser.error(f'Build the native {args.profile} executable first: missing {binary}')
subprocess.run([sys.executable, str(ROOT / 'scripts/bundle_resources.py'), 'package',
                '--target', args.target, '--output', str(args.output)], check=True)
with zipfile.ZipFile(args.output, 'a', compression=zipfile.ZIP_DEFLATED) as archive:
    info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
    info.external_attr = 0o100755 << 16
    info.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(info, binary.read_bytes())
    info = zipfile.ZipInfo('LICENSE', date_time=(2020, 1, 1, 0, 0, 0))
    info.external_attr = 0o100644 << 16
    info.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(info, (ROOT / 'LICENSE').read_bytes())
print(f'Packaged {args.output}; no formatter downloads are needed at runtime.')
