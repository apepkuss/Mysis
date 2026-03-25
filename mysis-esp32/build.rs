use std::fs;
use std::path::Path;

fn main() {
    // 编译期互斥检查
    let chips: &[(&str, &str)] = &[
        ("esp32s3", "sdkconfig.defaults.esp32s3"),
        ("esp32c3", "sdkconfig.defaults.esp32c3"),
        ("esp32c6", "sdkconfig.defaults.esp32c6"),
    ];

    let selected: Vec<&&str> = chips
        .iter()
        .filter(|(feat, _)| std::env::var(format!("CARGO_FEATURE_{}", feat.to_uppercase())).is_ok())
        .map(|(name, _)| name)
        .collect();

    if selected.len() != 1 {
        panic!(
            "Exactly one chip feature must be enabled. Found: {:?}",
            selected
        );
    }

    let chip = selected[0];
    let (_, sdkconfig_file) = chips.iter().find(|(name, _)| name == chip).unwrap();

    // 将芯片专属 sdkconfig 追加到公共 sdkconfig
    let common = fs::read_to_string("sdkconfig.defaults").expect("missing sdkconfig.defaults");
    let chip_specific =
        fs::read_to_string(sdkconfig_file).unwrap_or_else(|_| panic!("missing {sdkconfig_file}"));

    let merged = format!("{common}\n{chip_specific}");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let merged_path = Path::new(&out_dir).join("sdkconfig.defaults.merged");
    fs::write(&merged_path, merged).expect("failed to write merged sdkconfig");

    // 设置 ESP-IDF 使用合并后的 sdkconfig
    println!(
        "cargo:rustc-env=ESP_IDF_SDKCONFIG_DEFAULTS={}",
        merged_path.display()
    );

    embuild::espidf::sysenv::output();
}
