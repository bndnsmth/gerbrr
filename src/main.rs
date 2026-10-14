fn main() -> eframe::Result {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    // Keep the original flag spellings working, with friendlier gerbrr commands.
    if let Some(command) = args.first().and_then(|s| s.to_str()) {
        let alias = match command {
            "inspect" => Some("--inspect"),
            "screenshot" => Some("--screenshot"),
            "demo-export" => Some("--write-demo"),
            "help" => Some("--help"),
            "version" => Some("--version"),
            _ => None,
        };
        if let Some(alias) = alias {
            args[0] = alias.into();
        } else if matches!(command, "open" | "view") {
            args.remove(0);
        }
    }
    if args.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("gerbrr {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!(
            "Gerbrr — Gerber viewing at ridiculous speed.\n\nUsage: gerbrr [FILES | FOLDER | ZIP]\n       gerbrr open [FILES | FOLDER | ZIP]\n       gerbrr inspect [FILES | FOLDER | ZIP]\n       gerbrr screenshot FILE.png [FILES | FOLDER | ZIP]\n       gerbrr demo-export DIRECTORY\n       gerbrr --version\n\nNo arguments opens the built-in Atlas board.\nOriginal --inspect, --screenshot, and --write-demo flags also work.\nUse mise run dev for native, or mise run web for the browser PoC."
        );
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--write-demo") {
        let Some(directory) = args.get(1) else {
            eprintln!("Missing output directory");
            std::process::exit(2);
        };
        let directory = std::path::Path::new(directory);
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(directory)?;
            for (name, source) in gerbrr::document::demo_sources() {
                // Never overwrite existing fabrication files.
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.join(name))?;
                file.write_all(source.as_bytes())?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Export failed: {e}");
            std::process::exit(1);
        }
        println!("Wrote six example layers to {}", directory.display());
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--inspect") {
        let result = if args.len() == 1 {
            Ok(gerbrr::document::demo())
        } else {
            gerbrr::document::load(args[1..].iter().map(std::path::PathBuf::from).collect())
        };
        match result {
            Ok(document) => {
                println!(
                    "{}\n{} layers · {} primitives · {} triangles\n{:.2} × {:.2} mm · {:.1} MiB mesh · {:.2} ms import",
                    document.name,
                    document.layers.len(),
                    document.primitives(),
                    document.triangles(),
                    document.bounds.size()[0],
                    document.bounds.size()[1],
                    document.memory_bytes() as f64 / 1048576.0,
                    document.load_time.as_secs_f64() * 1000.0
                );
                for layer in &document.layers {
                    let dimensions = if layer.geometry.bounds.valid() {
                        format!(
                            "{:.2} × {:.2} mm",
                            layer.geometry.bounds.size()[0],
                            layer.geometry.bounds.size()[1]
                        )
                    } else {
                        "empty layer".into()
                    };
                    println!(
                        "  {}: {} · {} primitives · {} · {}",
                        layer.name,
                        layer.kind.label(),
                        layer.geometry.features.len(),
                        dimensions,
                        if layer.visible { "visible" } else { "hidden" }
                    );
                    for warning in &layer.warnings {
                        eprintln!("  Warning [{}]: {warning}", layer.name);
                    }
                }
                for error in &document.errors {
                    eprintln!("  Skipped: {error}");
                }
                if document.warning_count() > 0 {
                    std::process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let (paths, capture) = if args.first().is_some_and(|a| a == "--screenshot") {
        let Some(path) = args.get(1) else {
            eprintln!("Missing screenshot path");
            std::process::exit(2);
        };
        (
            args[2..].iter().map(std::path::PathBuf::from).collect(),
            Some(std::path::PathBuf::from(path)),
        )
    } else {
        if args.iter().any(|a| a.to_string_lossy().starts_with('-')) {
            eprintln!("Unknown option. Run --help for usage.");
            std::process::exit(2);
        }
        (args.iter().map(std::path::PathBuf::from).collect(), None)
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(format!(
                "{} — {}",
                gerbrr::brand::NAME,
                gerbrr::brand::TAGLINE
            ))
            .with_app_id("gerbrr")
            .with_icon(gerbrr::brand::icon())
            .with_resizable(capture.is_none())
            .with_inner_size([1440.0, 960.0])
            .with_min_inner_size([900.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        gerbrr::brand::NAME,
        options,
        Box::new(move |cc| Ok(Box::new(gerbrr::app::App::new(cc, paths, capture)))),
    )
}
