#!/usr/bin/env python3
"""Local-only, review-gated deterministic candidate packaging; no executable runs."""
import argparse
import hashlib
import json
import re
import shutil
import struct
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
HERE = Path(__file__).resolve().parent
HELPERS = HERE.parent
sys.path.insert(0, str(HELPERS))
from build_language_pack import EPOCH, digest_file, safe_path, write_bundle
TARGETS = frozenset({'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc'})
# Vue is a WASM adapter with a different target schema; use build_vue_pack.
TOOLS = frozenset({'gofmt', 'dartfmt'})
MAX_MEMBERS = 10000
MAX_MEMBER_BYTES = 512 * 1024 * 1024
MAX_PAYLOAD_BYTES = 1024 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
PROVENANCE = 'PROVENANCE.json'
def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + '\n').encode()
def sha(data): return hashlib.sha256(data).hexdigest()
def path_name(name):
    if not isinstance(name, str) or not name or len(name.encode()) > 100 or any(ord(c)<32 for c in name): raise ValueError('Invalid member path')
    safe_path(name)
    if name in {'.', '..'}: raise ValueError('Invalid member path')
    # Portable names: reject Windows-equivalent aliases even on Unix.
    for part in name.split('/'):
        if part.endswith((' ', '.')) or re.fullmatch(r'(?i)(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?',part): raise ValueError('Nonportable member path')
    return name

def validate_plan(plan):
    if not isinstance(plan, dict) or plan.get('available') is not False or plan.get('release_clearance') is not False:
        raise ValueError('Plan must explicitly remain unpublished and uncleared')
    if plan.get('tool') not in TOOLS or plan.get('target') not in TARGETS: raise ValueError('Unsupported tool/target')
    if not isinstance(plan.get('version'),str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._+-]{0,79}', plan['version']): raise ValueError('Invalid version')
    executable=path_name(plan.get('executable'))
    env=plan.get('environment')
    if not isinstance(env,dict) or any(not isinstance(k,str) or not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*',k) or not isinstance(v,str) or '\x00' in v for k,v in env.items()): raise ValueError('Invalid environment')
    provenance=plan.get('provenance')
    if not isinstance(provenance,dict) or not isinstance(provenance.get('sources'),list) or not provenance['sources'] or not isinstance(provenance.get('caveats'),list) or not provenance['caveats'] or any(not isinstance(c,str) or not c for c in provenance['caveats']): raise ValueError('Source provenance and caveats required')
    members=plan.get('members')
    if not isinstance(members,list) or not 0<len(members)<MAX_MEMBERS: raise ValueError('Invalid member count')
    names=set(); input_names=set(); folded=set(); total=0
    for m in members:
        if not isinstance(m,dict): raise ValueError('Invalid member')
        name=path_name(m.get('path')); inp=path_name(m.get('input_path'))
        if name.casefold()==PROVENANCE.casefold() or name.casefold() in folded or inp in input_names: raise ValueError('Duplicate/reserved path')
        folded.add(name.casefold())
        names.add(name); input_names.add(inp)
        if type(m.get('bytes')) is not int or not 0<=m['bytes']<=MAX_MEMBER_BYTES or type(m.get('executable')) is not bool: raise ValueError('Invalid member bounds/mode')
        if not isinstance(m.get('sha256'),str) or not re.fullmatch(r'[0-9a-f]{64}',m['sha256']): raise ValueError('Invalid hash')
        total+=m['bytes']
    if total>MAX_PAYLOAD_BYTES: raise ValueError('Payload limit exceeded')
    # Reject parent/file collisions, including reserved generated provenance.
    portable=folded|{PROVENANCE.casefold()}
    for name in portable:
        if any(str(p) in portable for p in PurePosixPath(name).parents if str(p)!='.'): raise ValueError('File/directory collision')
    if executable not in names or not next(m['executable'] for m in members if m['path']==executable): raise ValueError('Executable must be an executable pinned member')
    if len(encoded(plan))>8*1024*1024: raise ValueError('Plan metadata limit exceeded')  # also reject non-JSON/nonfinite
    return members

def provenance_bytes(plan):
    return encoded({'format_version':1,'purpose':'Local review candidate only; no release clearance or native acceptance implied.', 'available':False,'release_clearance':False,'plan_sha256':sha(encoded(plan)),'plan':plan})
def expected_members(plan):
    validate_plan(plan)
    members=[{k:m[k] for k in ('path','bytes','sha256','executable')} for m in plan['members']]
    data=provenance_bytes(plan)
    if len(data)>MAX_MEMBER_BYTES: raise ValueError('Provenance too large')
    members.append(dict(path=PROVENANCE,bytes=len(data),sha256=sha(data),executable=False))
    if sum(m['bytes'] for m in members)>MAX_PAYLOAD_BYTES: raise ValueError('Payload limit exceeded')
    return sorted([{**m,'archive_path':m['path']} for m in members],key=lambda m:m['path'])
def checked_regular(root,relative):
    root=Path(root).absolute()
    # Reject symlinked roots/ancestors instead of silently resolving them.
    for p in [root,*root.parents]:
        if p.is_symlink() or not p.is_dir(): raise ValueError('Unsafe input root/ancestor')
    path=root/path_name(relative)
    for p in path.parents:
        if p==root: break
        if p.is_symlink() or not p.is_dir(): raise ValueError('Unsafe input ancestor')
    if path.is_symlink() or not path.is_file(): raise ValueError('Input must be regular nonsymlink file')
    return path

def validate_candidate(plan,manifest,bundle):
    members=expected_members(plan); bundle=Path(bundle)
    if not isinstance(manifest,dict) or set(manifest)!={'tool','version','target','executable','environment','available','release_clearance','reason','max_installed_bytes','archives'}: raise ValueError('Unexpected manifest fields')
    if bundle.is_symlink() or not bundle.is_file(): raise ValueError('Bundle must be regular')
    for k in ('tool','version','target','executable','environment'):
        if manifest.get(k)!=plan[k]: raise ValueError('Manifest disagrees with plan: '+k)
    if manifest.get('available') is not False or manifest.get('release_clearance') is not False: raise ValueError('Manifest improperly enables release')
    if manifest.get('max_installed_bytes')!=sum(m['bytes'] for m in members): raise ValueError('Payload bound mismatch')
    archives=manifest.get('archives')
    if not isinstance(archives,list) or len(archives)!=1: raise ValueError('One archive required')
    a=archives[0]
    if not isinstance(a,dict) or set(a)!={'format','asset_name','bytes','sha256','max_download_bytes','max_expanded_bytes','members'}: raise ValueError('Unexpected archive fields')
    if 'url' in a or a.get('format')!='tar.gz' or a.get('members')!=members: raise ValueError('Archive plan mismatch or URL present')
    if a.get('asset_name')!=bundle.name or type(a.get('bytes')) is not int or not 0<a['bytes']<=MAX_ARCHIVE_BYTES or a.get('max_download_bytes')!=a['bytes'] or bundle.stat().st_size!=a['bytes'] or digest_file(bundle)!=a.get('sha256'): raise ValueError('Archive byte pin mismatch')
    expanded=sum(((m['bytes']+511)//512+1)*512 for m in members)+10240
    if a.get('max_expanded_bytes')!=expanded: raise ValueError('Expansion bound mismatch')
    with bundle.open('rb') as f: header=f.read(10)
    if len(header)!=10 or header[:4]!=b'\x1f\x8b\x08\x00' or struct.unpack('<I',header[4:8])[0]!=EPOCH: raise ValueError('Noncanonical gzip metadata')
    seen=[]
    with tarfile.open(bundle,'r:gz') as tar:
        for entry in tar:
            if len(seen)>=len(members): raise ValueError('Extra member')
            m=members[len(seen)]
            if entry.name!=m['path'] or not entry.isfile() or entry.size!=m['bytes'] or entry.mode!=(0o755 if m['executable'] else 0o644) or entry.uid!=0 or entry.gid!=0 or entry.uname or entry.gname or entry.mtime!=EPOCH or entry.pax_headers or entry.linkname: raise ValueError('Noncanonical member metadata')
            h=hashlib.sha256(); count=0
            with tar.extractfile(entry) as f:
                while chunk:=f.read(1024*1024): h.update(chunk); count+=len(chunk)
            if count!=m['bytes'] or h.hexdigest()!=m['sha256']: raise ValueError('Member content mismatch')
            seen.append(entry.name)
    if len(seen)!=len(members): raise ValueError('Missing member')

def build_candidate(plan,input_dir,output_dir):
    expected=expected_members(plan); output_dir=Path(output_dir)
    # An exclusive new output directory makes overwrite impossible.
    for p in [output_dir.absolute().parent,*output_dir.absolute().parent.parents]:
        if p.is_symlink() or not p.is_dir(): raise ValueError('Unsafe output ancestor')
    output_dir.mkdir(parents=False,exist_ok=False)
    try:
        with tempfile.TemporaryDirectory(prefix='stage-',dir=output_dir) as tmp:
            stage=Path(tmp)
            for m in plan['members']:
                src=checked_regular(input_dir,m['input_path'])
                if src.stat().st_size!=m['bytes'] or digest_file(src)!=m['sha256']: raise ValueError('Input pin mismatch')
                dst=stage/m['path']; dst.parent.mkdir(parents=True,exist_ok=True)
                with src.open('rb') as f, dst.open('xb') as o: shutil.copyfileobj(f,o,1024*1024)
                if dst.stat().st_size!=m['bytes'] or digest_file(dst)!=m['sha256']: raise ValueError('Input changed during copy')
            (stage/PROVENANCE).write_bytes(provenance_bytes(plan))
            name=f"codestyles-{plan['tool']}-{plan['version']}-{plan['target']}-review.tar.gz"
            bundle=output_dir/name; write_bundle(stage,bundle,expected)
            a=dict(format='tar.gz',asset_name=name,bytes=bundle.stat().st_size,sha256=digest_file(bundle),max_download_bytes=bundle.stat().st_size,max_expanded_bytes=sum(((m['bytes']+511)//512+1)*512 for m in expected)+10240,members=expected)
            manifest={k:plan[k] for k in ('tool','version','target','executable','environment')}
            manifest.update(available=False,release_clearance=False,reason='Local review candidate; release and target acceptance remain separate.',max_installed_bytes=sum(m['bytes'] for m in expected),archives=[a])
            validate_candidate(plan,manifest,bundle)
            with (output_dir/'candidate-manifest.json').open('xb') as f: f.write(encoded(manifest))
            return manifest
    except BaseException:
        shutil.rmtree(output_dir)
        raise
def load_reviewed_plan(path, expected_sha256):
    path = Path(path)
    if not isinstance(expected_sha256, str) or not re.fullmatch(r'[0-9a-f]{64}', expected_sha256):
        raise ValueError('A full reviewed plan SHA-256 is required')
    if path.is_symlink() or not path.is_file():
        raise ValueError('Plan must be regular')
    if digest_file(path) != expected_sha256:
        raise ValueError('Plan hash mismatch')
    return json.loads(path.read_text())
if __name__=='__main__':
    p=argparse.ArgumentParser(); p.add_argument('--plan',type=Path,required=True); p.add_argument('--input-dir',type=Path,required=True); p.add_argument('--output-dir',type=Path,required=True); p.add_argument('--plan-sha256',required=True); a=p.parse_args()
    m=build_candidate(load_reviewed_plan(a.plan,a.plan_sha256),a.input_dir,a.output_dir); print(json.dumps({'archive':m['archives'][0]['asset_name'],'sha256':m['archives'][0]['sha256']}))
