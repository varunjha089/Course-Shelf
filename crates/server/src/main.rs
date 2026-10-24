//! courseshelf-serve: run the viewer server without the Mac app (for a browser or for testing).
//!   courseshelf-serve [--port 8765] [--ui web/dist]

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port = 0u16;
    let mut ui = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--ui" => {
                ui = args.get(i + 1).map(std::path::PathBuf::from);
                i += 1;
            }
            "-h" | "--help" => {
                println!("courseshelf-serve [--port N] [--ui DIR]\nOpens the printed URL in a browser. The data folder is $COURSESHELF_HOME, else the folder named in home.txt in the settings folder, else the settings folder.");
                return Ok(());
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
        i += 1;
    }
    let home = courseshelf_core::config::default_home();
    for f in courseshelf_core::config::ensure_home(&home)? {
        println!("first start: wrote {}", f.display());
    }
    let run = courseshelf_server::start_background(home.clone(), port, ui)?;
    println!("CourseShelf serving {}\nOpen: {}", home.display(), run.url());
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
