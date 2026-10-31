import copy, errno, gzip, hashlib, io, json, struct, tarfile, tempfile, unittest
from pathlib import Path
from candidate_pipeline import build_candidate as b
class CandidateTests(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory(dir=b.HERE); self.root=Path(self.temp.name); self.inputs=self.root/'inputs';self.inputs.mkdir();(self.inputs/'formatter').write_bytes(b'fixture-not-an-executable\n'); (self.inputs/'NOTICE').write_text('fixture notice\n')
  self.plan=dict(tool='gofmt',version='1.27.3',target='x86_64-unknown-linux-gnu',executable='bin/gofmt',environment={'GOTELEMETRY':'off'},available=False,release_clearance=False,provenance={'sources':[{'kind':'fixture'}],'caveats':['Fixture only; no runtime validation.']},members=[dict(input_path=n,path=p,bytes=(self.inputs/n).stat().st_size,sha256=b.digest_file(self.inputs/n),executable=e) for n,p,e in [('formatter','bin/gofmt',True),('NOTICE','licenses/NOTICE',False)]])
 def symlink(self, path, target, directory=False):
  try:path.symlink_to(target,target_is_directory=directory)
  except OSError as error:
   if getattr(error,'winerror',None)==1314 or error.errno in (errno.EPERM,errno.ENOSYS,errno.EOPNOTSUPP):self.skipTest('Symlink creation unavailable on this runner')
   raise
 def tearDown(self):self.temp.cleanup()
 def build(self,plan=None,name='out'):return b.build_candidate(plan or self.plan,self.inputs,self.root/name)
 def test_deterministic_and_validate_api(self):
  a=self.build(name='one'); c=self.build(name='two');self.assertEqual(a,c);b.validate_candidate(self.plan,a,self.root/'one'/a['archives'][0]['asset_name']);self.assertEqual(a['environment'],self.plan['environment']);self.assertNotIn('url',a['archives'][0])
 def test_overwrite_rejected(self):
  self.build();before=(self.root/'out'/'candidate-manifest.json').read_bytes()
  with self.assertRaises(FileExistsError):self.build()
  self.assertEqual(before,(self.root/'out'/'candidate-manifest.json').read_bytes())
 def test_symlink_output_parent_rejected(self):
  alias=self.root/'alias';self.symlink(alias,self.root,True)
  with self.assertRaisesRegex(ValueError,'output ancestor'): b.build_candidate(self.plan,self.inputs,alias/'out')
 def test_corrupt_input(self):
  (self.inputs/'formatter').write_bytes(b'bad')
  with self.assertRaisesRegex(ValueError,'Input pin'):self.build()
  self.assertFalse((self.root/'out').exists())
 def test_published_plan(self):
  for key in ['available','release_clearance']:
   p=copy.deepcopy(self.plan);p[key]=True
   with self.assertRaises(ValueError):self.build(p)
 def test_paths(self):
  for n in ['../escape','/absolute','x\\y','a:b','a/../b','./x','.', 'bad\x00name','NUL.txt','trailing.','trailing ']:
   p=copy.deepcopy(self.plan);p['members'][1]['path']=n
   with self.assertRaises(ValueError,msg=n):self.build(p)
 def test_duplicate_reserved_collision(self):
  for n in ['bin/gofmt','bin','PROVENANCE.json','PROVENANCE.json/child','BIN/GOFMT','BIN','provenance.json/child']:
   p=copy.deepcopy(self.plan);p['members'][1]['path']=n
   with self.assertRaises(ValueError,msg=n):self.build(p)
 def test_input_symlink_and_ancestor(self):
  (self.inputs/'formatter').unlink();self.symlink(self.inputs/'formatter',self.inputs/'NOTICE')
  with self.assertRaisesRegex(ValueError,'nonsymlink'):self.build()
  (self.inputs/'formatter').unlink(); self.symlink(self.inputs/'linked',self.inputs,True);p=copy.deepcopy(self.plan);p['members'][0]['input_path']='linked/NOTICE'
  with self.assertRaisesRegex(ValueError,'ancestor'):self.build(p)
 def test_nested_archive_path_collision_is_host_independent(self):
  p=copy.deepcopy(self.plan);p['members'][0]['path']='nested/bin/gofmt';p['executable']='nested/bin/gofmt';p['members'][1]['path']='nested/bin'
  with self.assertRaisesRegex(ValueError,'collision'):b.validate_plan(p)
 def test_unsupported_target_and_tool(self):
  for k,v in [('target','wasm32-unknown-unknown'),('tool','arbitrary')]:
   p=copy.deepcopy(self.plan);p[k]=v
   with self.assertRaises(ValueError):self.build(p)
 def test_vue_wasm_adapter_rejected(self):
  p=copy.deepcopy(self.plan);p['tool']='vue'
  with self.assertRaisesRegex(ValueError,'Unsupported tool'):self.build(p)
 def test_bounds(self):
  p=copy.deepcopy(self.plan);p['members'][0]['bytes']=b.MAX_MEMBER_BYTES+1
  with self.assertRaises(ValueError):self.build(p)
 def test_manifest_cannot_enable_or_change_env(self):
  m=self.build();bundle=self.root/'out'/m['archives'][0]['asset_name']
  for k,v in [('available',True),('release_clearance',True),('environment',{})]:
   x=copy.deepcopy(m);x[k]=v
   with self.assertRaises(ValueError):b.validate_candidate(self.plan,x,bundle)
 def test_manifest_url_rejected(self):
  m=self.build();bundle=self.root/'out'/m['archives'][0]['asset_name'];m['url']='https://example.invalid/unapproved'
  with self.assertRaisesRegex(ValueError,'Unexpected manifest'):b.validate_candidate(self.plan,m,bundle)
 def test_archive_corruption(self):
  m=self.build();bundle=self.root/'out'/m['archives'][0]['asset_name'];raw=bytearray(bundle.read_bytes());raw[-1]^=1;bundle.write_bytes(raw)
  with self.assertRaises(ValueError):b.validate_candidate(self.plan,m,bundle)
 def test_archive_metadata_even_if_hash_updated(self):
  m=self.build();bundle=self.root/'out'/m['archives'][0]['asset_name'];raw=bytearray(bundle.read_bytes());raw[4:8]=struct.pack('<I',1);bundle.write_bytes(raw);m['archives'][0]['sha256']=b.digest_file(bundle)
  with self.assertRaisesRegex(ValueError,'gzip metadata'):b.validate_candidate(self.plan,m,bundle)
 def test_member_mode_even_if_archive_hash_updated(self):
  m=self.build();bundle=self.root/'out'/m['archives'][0]['asset_name']; files=[]
  with tarfile.open(bundle,'r:gz') as t:
   for e in t: files.append((e,t.extractfile(e).read()))
  files[0][0].mode=0o777
  with bundle.open('wb') as f,gzip.GzipFile(filename='',fileobj=f,mode='wb',mtime=b.EPOCH) as g,tarfile.open(fileobj=g,mode='w|',format=tarfile.USTAR_FORMAT) as t:
   for e,data in files:t.addfile(e,io.BytesIO(data))
  a=m['archives'][0];a['bytes']=a['max_download_bytes']=bundle.stat().st_size;a['sha256']=b.digest_file(bundle)
  with self.assertRaisesRegex(ValueError,'metadata'):b.validate_candidate(self.plan,m,bundle)
if __name__=='__main__':unittest.main()
