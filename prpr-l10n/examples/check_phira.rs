fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../phira/locales/");
    prpr_l10n::tools::check_langfile(path).expect("Phira localization validation failed");
}
