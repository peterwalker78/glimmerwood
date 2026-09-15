mod asking;
mod attention;
mod bookmarks;
mod chrome;
mod companion;
mod diary;
mod dose;
mod failure;
mod feel_lab;
mod find;
mod home;
mod look;
mod nav;
mod oklab;
mod places;
mod prefs;
mod protocol;
mod ratings;
mod reputation;
mod scheme;
mod settings;
mod store;
mod tabs;
mod window;
mod wisp_view;

use std::cell::OnceCell;
use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

const APP_ID: &str = "io.github.peterwalker78.Glimmerwood";

fn main() -> glib::ExitCode {
    let resources = gio::Resource::from_data(&glib::Bytes::from_static(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/glimmerwood.gresource"
    ))))
    .expect("bundled resources are valid");
    gio::resources_register(&resources);

    let (args, lab) = match feel_lab_args(std::env::args().collect()) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("glimmerwood: {err}");
            return glib::ExitCode::FAILURE;
        }
    };
    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if lab.is_some() {
        // A lab runs beside a real Glimmerwood, never inside it.
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let lab = Rc::new(std::cell::RefCell::new(lab));
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();

    let companion: Rc<OnceCell<Rc<companion::Companion>>> = Rc::default();

    app.connect_startup(glib::clone!(
        #[strong]
        companion,
        move |app| {
            scheme::register();
            if let Some(session) = webkit::NetworkSession::default() {
                // Tracking prevention is computed on this machine; it sends nothing.
                session.set_itp_enabled(true);
                // Site icons for the tab column, kept with the rest of the
                // browsing data.
                if let Some(data) = session.website_data_manager() {
                    data.set_favicons_enabled(true);
                }
            }
            follow_system_colour_scheme();
            window::install_accels(app);
            let _ = companion.set(companion::Companion::new(lab.borrow_mut().take()));
        }
    ));

    app.connect_activate(glib::clone!(
        #[strong]
        companion,
        move |app| {
            let companion = companion.get().expect("set at startup");
            window::Window::new(app, companion).present();
        }
    ));

    // Arguments are taken as if typed into the address field, one tab each.
    // GApplication's "open" would turn a bare `example.org` into a file path.
    app.connect_command_line(move |app, command_line| {
        let companion = companion.get().expect("set at startup");
        let window = window::Window::new(app, companion);
        let args = command_line.arguments();
        for (i, arg) in args.iter().skip(1).enumerate() {
            window.open(&arg.to_string_lossy(), i > 0);
        }
        window.present();
        glib::ExitCode::SUCCESS
    });

    app.run_with_args(&args)
}

/// Take `--feel-lab`, `--from=HH:MM`, `--speed=N` and `--show-caption` out
/// of the arguments.
fn feel_lab_args(args: Vec<String>) -> Result<(Vec<String>, Option<feel_lab::Lab>), String> {
    let mut rest = Vec::new();
    let (mut lab, mut from, mut speed, mut caption) = (false, None, 60.0, false);
    for arg in args {
        if arg == "--feel-lab" {
            lab = true;
        } else if arg == "--show-caption" {
            caption = true;
        } else if let Some(time) = arg.strip_prefix("--from=") {
            from = Some(time.to_owned());
        } else if let Some(n) = arg.strip_prefix("--speed=") {
            speed = n
                .parse()
                .map_err(|_| format!("--speed={n} is not a number"))?;
        } else {
            rest.push(arg);
        }
    }
    if !lab {
        return Ok((rest, None));
    }
    let mut lab = feel_lab::Lab::new(feel_lab::DAY, attention::now(), speed, from.as_deref())?;
    lab.show_caption = caption;
    Ok((rest, Some(lab)))
}

/// Inside the Flatpak sandbox GTK learns the desktop's light/dark preference
/// from the settings portal, but WebKit only looks at the older
/// prefer-dark-theme setting, so pages and the chrome would stay light. Copy
/// the preference across, and keep copying it when it changes.
fn follow_system_colour_scheme() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    // Deprecated in GTK 4.20 in favour of the interface colour scheme, but it
    // is still what WebKitGTK 2.52 consults for prefers-color-scheme.
    #[allow(deprecated)]
    let apply = |settings: &gtk::Settings| match settings.gtk_interface_color_scheme() {
        gtk::InterfaceColorScheme::Dark => settings.set_gtk_application_prefer_dark_theme(true),
        gtk::InterfaceColorScheme::Light => settings.set_gtk_application_prefer_dark_theme(false),
        // No preference from the platform: leave whatever GTK's own config says.
        _ => {}
    };
    apply(&settings);
    settings.connect_gtk_interface_color_scheme_notify(apply);
}
