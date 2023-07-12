mod app;

fn main() -> anyhow::Result<()> {
    let mut app = app::App::new();
    init_logs(app.log_level_filter());
    app.work()?;

    Ok(())
}

fn init_logs(log_level: log::LevelFilter) {
    let colors = fern::colors::ColoredLevelConfig::default()
        .info(fern::colors::Color::Blue)
        .debug(fern::colors::Color::Yellow)
        .trace(fern::colors::Color::Magenta);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{color_line}{}[{}][{}{color_line}]\x1B[0m {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                colors.color(record.level()),
                message,
                color_line =
                    format_args!("\x1B[{}m", colors.get_color(&record.level()).to_fg_str())
            ))
        })
        .level(log_level)
        .chain(std::io::stdout())
        .apply()
        .unwrap();
}
