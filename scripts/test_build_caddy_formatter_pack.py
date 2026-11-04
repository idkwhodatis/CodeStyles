"""Offline archive integrity and native formatter contract tests."""
import hashlib,io,json,tarfile,tempfile,unittest,zipfile
from pathlib import Path
import build_caddy_formatter_pack as b

class CaddyTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.plan=json.loads(b.SOURCES.read_text())
    def fixture(self,target,entries=None):
        pin=self.plan['targets'][target];p=self.root/'source';entries=entries or [(pin['binary'],b'binary'),('LICENSE',b'license')]
        if pin['archive_format']=='zip':
            with zipfile.ZipFile(p,'w') as a:
                for name,data in entries:a.writestr(name,data)
        else:
            with tarfile.open(p,'w:gz') as a:
                for name,data in entries:
                    info=tarfile.TarInfo(name);info.size=len(data);a.addfile(info,io.BytesIO(data))
        pin['bytes']=p.stat().st_size;pin['sha256']=hashlib.sha256(p.read_bytes()).hexdigest()
        s=self.root/'sources.json';s.write_text(json.dumps(self.plan));return p,s
    def test_all_six_deterministic_disabled_native_candidates(self):
        for target in self.plan['targets']:
            p,s=self.fixture(target);one=self.root/'one.tar.gz';two=self.root/'two.tar.gz'
            candidate=b.build(target,p,one,s);b.build(target,p,two,s)
            self.assertEqual(one.read_bytes(),two.read_bytes());self.assertIs(candidate['available'],False)
            self.assertNotIn('wasm_plugins',candidate);self.assertNotIn('url',candidate['archives'][0]);self.assertEqual(candidate['environment'],{})
            with tarfile.open(one) as a:
                self.assertEqual(json.load(a.extractfile('PROVENANCE.json'))['invocation'],['fmt','-'])
                for m in candidate['archives'][0]['members']:
                    self.assertEqual(hashlib.sha256(a.extractfile(m['path']).read()).hexdigest(),m['sha256'])
                    self.assertEqual(a.getmember(m['path']).mode,0o755 if m['executable'] else 0o644)
    def test_corrupt_source_fails(self):
        target=next(iter(self.plan['targets']));p,s=self.fixture(target);p.write_bytes(b'bad')
        with self.assertRaisesRegex(ValueError,'mismatch'):b.build(target,p,self.root/'out',s)
        self.assertFalse((self.root/'out').exists())
    def test_missing_and_duplicate_members_fail(self):
        target=next(iter(self.plan['targets']));name=self.plan['targets'][target]['binary']
        for entries in [[(name,b'bin')],[(name,b'a'),(name,b'b'),('LICENSE',b'notice')]]:
            p,s=self.fixture(target,entries)
            with self.assertRaisesRegex(ValueError,'Missing|Duplicate'):b.build(target,p,self.root/'out',s)
    def test_archive_links_rejected(self):
        target=next(iter(self.plan['targets']));p,s=self.fixture(target)
        with tarfile.open(p,'w:gz') as a:
            item=tarfile.TarInfo(self.plan['targets'][target]['binary']);item.type=tarfile.SYMTYPE;item.linkname='/etc/passwd';a.addfile(item)
        pin=self.plan['targets'][target];pin['bytes']=p.stat().st_size;pin['sha256']=hashlib.sha256(p.read_bytes()).hexdigest();s.write_text(json.dumps(self.plan))
        with self.assertRaisesRegex(ValueError,'regular'):b.build(target,p,self.root/'out',s)
    def test_catalog_is_disabled_and_pinned(self):
        tool=json.loads((b.RESOURCES/'toolchains.json').read_text())['toolchains']['caddyfmt']
        self.assertEqual(tool['version'],self.plan['version']);self.assertEqual(set(tool['targets']),set(self.plan['targets']))
        for target,spec in tool['targets'].items():
            self.assertIs(spec['available'],False);self.assertNotIn('wasm_plugins',spec)
            self.assertEqual(spec['executable'],'bin/'+self.plan['targets'][target]['binary'])
            self.assertNotIn('url',spec['archives'][0])
if __name__=='__main__':unittest.main()
