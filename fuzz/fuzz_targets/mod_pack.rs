#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("mod_pack", data);
    let _ = d2_data::patch::parse_stack("synthetic.d2stack", data);
    let _ = d2_data::patch::parse_layer("synthetic.d2patch", data, 0);
    let _ = d2_data::patch::load_stack("synthetic.d2stack", data, &mut |_| Some(data.to_vec()));
    if let Ok(text) = std::str::from_utf8(data) {
        for lenient in [false, true] {
            let _ = d2_native::manifest::parse_files_tsv(text, lenient);
        }
        let _ = d2_native::layers::ModInfo::from_toml("mod.toml", text);
        let _ = d2_native::layers::parse_order(text);
        if let Ok(manifest) = d2_native::manifest::Manifest::from_toml(text) {
            let _ = manifest.check_loadable(&Default::default());
        }
        let files: Vec<String> = text.lines().take(1024).map(str::to_owned).collect();
        let _ = d2_native::layers::check_mod_files("synthetic", &files);
    }
});
