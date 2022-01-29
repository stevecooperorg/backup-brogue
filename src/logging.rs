use fern::colors::{Color, ColoredLevelConfig};

pub fn setup_logger() -> std::result::Result<(), fern::InitError> {
    let mut colors = ColoredLevelConfig::new()
        // use builder methods
        .info(Color::Green);
    // or access raw fields
    colors.warn = Color::Magenta;

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{}[{}] {}",
                chrono::Local::now().format("[w%Y-%m-%d %H:%M:%S.%3f]"),
                colors.color(record.level()),
                message
            ))
        })
        //.level(log::LevelFilter::Debug)
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout())
        .chain(fern::log_file("output.log")?)
        .apply()?;
    Ok(())
}

#[macro_export]
macro_rules! notify {
    ($($arg:tt)*) => ({
        let msg: String = format!($($arg)*);
        Notification::new()
            .summary("backup-brogue")
            .body(&msg)
            .show()
        .expect("could not show notification");
    })
}
