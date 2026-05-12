//! NativeActivity entry point. The `#[unsafe(no_mangle)]` attribute requires
//! a relaxed lint posture (the crate root uses `deny(unsafe_code)` instead of
//! `forbid`); this is the single file that needs the carve-out.

#![allow(unsafe_code)]

use winit::platform::android::activity::AndroidApp;

use crate::android::app::LportfolioApp;
use crate::android::data_dir;

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("lportfolio"),
    );

    if let Some(path) = app.internal_data_path() {
        data_dir::set(path);
    }

    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
        "lportfolio",
        options,
        Box::new(|_cc| Ok(Box::new(LportfolioApp::new()))),
    ) {
        log::error!("eframe::run_native failed: {e:?}");
    }
}
