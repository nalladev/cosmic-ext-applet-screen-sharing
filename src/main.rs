// SPDX-License-Identifier: MPL-2.0

mod activation;
mod app;
mod config;
mod fcast;
mod i18n;

use app::Flags;

/// Print usage information for the command-line interface.
fn print_help() {
    println!(
        "Usage: cosmic-ext-applet-screen-sharing [OPTIONS]\n\
         \n\
         COSMIC screen sharing applet — share your screen to wired displays\n\
         and wireless receivers.\n\
         \n\
         Options:\n\
         \x20 -h, --help   Show this help and exit."
    );
}

fn main() -> cosmic::iced::Result {
    // Set up leveled logging (stderr → journald in production).  Users can
    // override with RUST_LOG, e.g. RUST_LOG=cosmic_ext_applet_screen_sharing=debug.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "cosmic_ext_applet_screen_sharing=info,wgpu=warn,cosmic=warn,iced=warn",
    ))
    .try_init();

    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Parse command-line arguments (--help).
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                log::error!("unknown argument: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }

    // Starts the applet's event loop with the parsed flags.
    cosmic::applet::run::<app::AppModel>(Flags)
}
