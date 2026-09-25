"""Verify the drawing body is exactly r14 after undoing the render-kind input rename."""
from pathlib import Path
import hashlib
source = (Path(__file__).resolve().parents[1] / 'prpr/src/core/note.rs').read_text()
a = source.index('    fn render_with_kind(')
b = source.index('\n}\n\npub struct BadNote', a)
body = source[a:b].replace('fn render_with_kind(&self, kind: NoteKind, res:', 'pub fn render(&self, res:')
import re
body = re.sub(r'\bkind\b', 'self.kind', body)
assert hashlib.sha256(body.encode()).hexdigest() == '560687b1d86166e772d69b29eba3b5ce64778c5aed90417ae9fc4471430069c1', 'r14 drawing implementation changed'
print('PASS: drawing body exactly matches r14; only render-kind input was substituted')
