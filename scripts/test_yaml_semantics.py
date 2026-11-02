"""Opt-in real pinned Pretty YAML semantic regression tests (no downloads).

Set CODESTYLES_YAML_SEMANTIC_DPRINT and CODESTYLES_YAML_SEMANTIC_WASM to
already-provisioned pinned files. PyYAML is an optional test oracle, not an app
runtime dependency. Its YAML 1.1 interpretation is not a universal YAML 1.2
conformance claim; the assertions compare the same parser before and after.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

try:
    import yaml
except ImportError:
    yaml = None

ROOT = Path(__file__).resolve().parent.parent
HOST = os.environ.get('CODESTYLES_YAML_SEMANTIC_DPRINT')
PLUGIN = os.environ.get('CODESTYLES_YAML_SEMANTIC_WASM')
KEEP_CHOMPING_REGRESSION = 'text: >+\n  first paragraph\n  still first paragraph\n\n  second paragraph\n\n'

FIXTURES = {
    'block_scalar_trailing_spaces': 'literal: |\n  line  \n  next \nfolded: >\n  line  \n  next \n',
    'anchors_comments_multiline': '''# Keep this leading comment
base: &defaults
  retries: 3 # Keep this inline comment
  enabled: true
copy: *defaults
merged: {<<: *defaults, name: 'literal'}
literal: |-
  first line
    extra indent
  final line
folded: >
  first paragraph
  still first paragraph

  second paragraph

''',
    'ambiguous_scalars': '''values: [yes, no, on, off, null, ~, true, false, 001, 1.0, 1e3, 2024-01-01]
quoted: ['yes', "no", 'null', '001', "1.0", '2024-01-01', 'a: b', 'hash # literal']
empty: ''
colon: 'x: y'
''',
    'documents_and_flow': '''---
items: [{name: 'one', value: 1}, {name: "two", value: null}]
...
---
- 'string'
- [true, false, null]
- {nested: {answer: 42}}
''',
}


def typed_value(value):
    """Do not let Python equate bool/int/float or negative/positive zero."""
    if isinstance(value, dict):
        return ('dict', sorted((typed_value(k), typed_value(v)) for k, v in value.items()))
    if isinstance(value, list):
        return ('list', [typed_value(item) for item in value])
    return (type(value).__name__, repr(value))


@unittest.skipUnless(yaml is not None and HOST and PLUGIN,
                     'requires PyYAML and explicit pre-provisioned pinned host/WASM paths')
class YamlSemanticTests(unittest.TestCase):
    def test_pinned_engine_preserves_values_aliases_comments_and_block_scalars(self):
        plugin = Path(PLUGIN).resolve(strict=True)
        self.assertEqual(hashlib.sha256(plugin.read_bytes()).hexdigest(),
                         '40a2fdda7040317eb1b23520f3a00769a5571eedb049c4ca9175c1b9eeba01ae')
        host = Path(HOST).resolve(strict=True)
        digest = hashlib.sha256(host.read_bytes()).hexdigest()
        catalog = json.loads((ROOT / 'resources/toolchains.json').read_text())
        host_hashes = {member['sha256']
                       for spec in catalog['toolchains']['dprint']['targets'].values()
                       for archive in spec.get('archives', [])
                       for member in archive.get('members', [])
                       if member['path'] in ('bin/dprint', 'bin/dprint.exe')}
        self.assertIn(digest, host_hashes)
        with tempfile.TemporaryDirectory(prefix='yaml-semantics-') as directory:
            root = Path(directory)
            config = root / 'dprint.json'
            env = os.environ | {'DPRINT_NO_UPDATE_CHECK': '1'}
            env.setdefault('DPRINT_CACHE_DIR', str(root / 'cache'))
            for width in (2, 4):
                config.write_text(json.dumps({'plugins': [str(plugin)], 'yaml': {'indentWidth': width, 'trimTrailingWhitespaces': False}}))
                for name, source in FIXTURES.items():
                    with self.subTest(name=name, indent_width=width):
                        command = [str(host), 'fmt', '--config-discovery=false', '--config', str(config), '--stdin=sample.yaml']
                        result = subprocess.run(command, input=source, text=True, capture_output=True,
                                                check=True, timeout=30, env=env)
                        output = result.stdout
                        self.assertEqual(typed_value(list(yaml.safe_load_all(source))),
                                         typed_value(list(yaml.safe_load_all(output))))
                        second = subprocess.run(command, input=output, text=True, capture_output=True,
                                                check=True, timeout=30, env=env)
                        self.assertEqual(second.stdout, output)
                        if name == 'anchors_comments_multiline':
                            for marker in ('&defaults', '*defaults', '# Keep this leading comment', '# Keep this inline comment'):
                                self.assertIn(marker, output)
                            parsed = yaml.safe_load(output)
                            self.assertIs(parsed['base'], parsed['copy'])

            # Upstream's default also changes significant trailing spaces;
            # the adapter therefore forces false and rejects explicit true.
            config.write_text(json.dumps({'plugins': [str(plugin)], 'yaml': {}}))
            source = FIXTURES['block_scalar_trailing_spaces']
            result = subprocess.run(command, input=source, text=True, capture_output=True,
                                    check=True, timeout=30, env=env)
            self.assertNotEqual(typed_value(yaml.safe_load(source)),
                                typed_value(yaml.safe_load(result.stdout)))
            config.write_text(json.dumps({'plugins': [str(plugin)],
                                          'yaml': {'trimTrailingWhitespaces': False}}))
            # Pin the reason the adapter rejects keep-chomping. This is an
            # upstream regression witness, not acceptance of altered semantics.
            for indicator in ('|+', '>+', '|2+', '>2+', '|+2', '>+2'):
                source = KEEP_CHOMPING_REGRESSION.replace('>+', indicator, 1)
                result = subprocess.run(command, input=source, text=True, capture_output=True,
                                        check=True, timeout=30, env=env)
                self.assertNotEqual(yaml.safe_load(source), yaml.safe_load(result.stdout), indicator)


if __name__ == '__main__':
    unittest.main()
