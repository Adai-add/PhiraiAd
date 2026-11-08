# Library RKS gain hints

Run the production calculator tests and RKS editor integration tests without a GPU:

```sh
cargo test --manifest-path tests/rks-gain/Cargo.toml --locked
```

This standalone harness imports `phira/src/custom_rks.rs` and the existing
`tests/chart-play/tests/rks_ui.rs` adapters. It does not depend on the unrelated
chart-play library adapters. Eleven calculator tests also run inside the editor
test binary; those are repeated checks, not additional unique cases. Absolute
accuracy targets are checked against the production RKS calculation, including
strict ties and current records with more than two decimal places.

Run the production hint renderer with an OpenGL display and the repository assets:

```sh
cargo run --locked -p phira --example rks_gain_hint_regression
```

The graphical check draws 28 rows across 3/4/6/8-column card widths, checks
spacing and bounds, and reads rendered pixels to check glyphs and colors.

Release metadata checks:

```sh
python3 scripts/tests/test_release_version.py
pwsh -NoProfile -File scripts/tests/test_windows_release.ps1
pwsh -NoProfile -File scripts/tests/test_windows_release.ps1 -BuildScriptsDirectory PhiraiAd-build-all-new-v1-modified/scripts
```

The PowerShell fixture uses real packaging logic and ZIP/manifest validation,
with native build commands stubbed out. It does not validate a Windows binary.
