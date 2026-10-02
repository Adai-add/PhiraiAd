#!/usr/bin/env python3
"""Check that all timing HUD/editor source files were overlaid together."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = {
    'prpr/src/lib.rs': ('pub mod timing_bar;', 'pub mod timing_preview;'),
    'prpr/src/scene/game.rs': ('pub fn render_timing_preview(', 'preview_combo: Option<u32>'),
    'prpr/src/timing_preview.rs': ('pub async fn load(',),
    'prpr/src/timing_bar.rs': ('last_input_time:', 'last_three:', 'config.retention_seconds()'),
    'prpr/src/config.rs': ('pub record_seconds: f32',),
    'phira/src/page/timing_editor.rs': ('scene.render_timing_preview(', 'self.draft.record_seconds'),
}
failed = False
for relative, tokens in checks.items():
    path = root / relative
    content = path.read_text(encoding='utf-8') if path.is_file() else ''
    missing = [token for token in tokens if token not in content]
    print(('FAIL' if missing else 'OK') + ': ' + str(path))
    if missing:
        failed = True
        print('  missing: ' + ', '.join(missing))
if failed:
    print('Overlay ALL files from the v3 modified ZIP into this source root, then run this check again.')
    sys.exit(1)
print('Source overlay is complete. Clean prpr/phira Android release artifacts and rebuild.')
