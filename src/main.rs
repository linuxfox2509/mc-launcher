use clap::Parser;
use std::io::{self, Write};

#[derive(Parser)]
#[command(
    name = "Minecraft Launcher",
    version = "1.0",
    about = "Launch Minecraft using the command line"
)]
struct Args {
    #[arg(short, long, help = "Show all versions including snapshots etc.")]
    all: bool,
    #[arg(short = 'i', long = "ver", help = "Select version by ID (from Index)")]
    selected_version: Option<usize>,
    #[arg(short, long, help = "Set username")]
    username: Option<String>,
}

mod utils;

fn main() {
    let args = Args::parse();
    let show_all = args.all;
    println!("Minecraft Launcher (Rust)");

    // Fetch available versions with URLs
    let versions = match utils::fetch_versions_with_urls() {
        Ok(v) => {
            if args.all {
                v // show everything
            } else {
                v.into_iter()
                    .filter(|(id, _)| id.starts_with("1.") && !id.contains("snapshot"))
                    .collect()
            }
        }
        Err(e) => {
            eprintln!("Failed to fetch versions: {}", e);
            return;
        }
    };

    println!("Available versions:");
    for (i, (id, _url)) in versions.iter().enumerate() {
        println!("{}: {}", i + 1, id);
    }

    if !args.all {
        println!("(Showing only major versions. Use --all or -a to include snapshots.)");
    }

    print!("Select a version by number: ");
    io::stdout().flush().unwrap();

    let choice = match args.selected_version {
        Some(index) => index,
        None => {
            print!("Select a version by number: ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            input.trim().parse().unwrap_or(0)
        }
    };

    if choice == 0 || choice > versions.len() {
        println!("Invalid choice.");
        return;
    }

    let (selected_version, version_url) = &versions[choice - 1];
    println!("Selected version: {}", selected_version);

    // Detect Java installations
    let java_paths = utils::detect_java();
    if java_paths.is_empty() {
        println!("No Java installations found. Please install Java.");
        return;
    }

    println!("Found Java installations:");
    for path in &java_paths {
        println!("{}", path);
    }

    // Download the selected version files
    println!("Downloading Minecraft version files...");
    match utils::download_version_files(selected_version, version_url) {
        Ok(_) => println!("Download complete!"),
        Err(e) => {
            eprintln!("Failed to download files: {}", e);
            return;
        }
    }

    // Define jar_path here
    let jar_path = format!("minecraft/{}.jar", selected_version);

    // Parse version JSON
    let version_json_path = format!("minecraft/{}.json", selected_version);
    let version_json_str =
        std::fs::read_to_string(&version_json_path).expect("Failed to read version JSON");
    let version_detail: utils::VersionDetail =
        serde_json::from_str(&version_json_str).expect("Failed to parse version JSON");

    // Download libraries
    println!("Downloading libraries...");
    let lib_paths = match utils::download_libraries(&version_detail.libraries) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("Failed to download libraries: {}", e);
            return;
        }
    };

    // Extract native libraries
    println!("Extracting native libraries...");
    match utils::extract_natives(&version_detail.libraries) {
        Ok(_) => println!("Natives extracted!"),
        Err(e) => {
            eprintln!("Failed to extract natives: {}", e);
            return;
        }
    }

    // Download assets
    println!("Downloading assets...");
    match utils::download_assets(&version_detail.assetIndex) {
        Ok(_) => println!("Assets downloaded!"),
        Err(e) => {
            eprintln!("Failed to download assets: {}", e);
            return;
        }
    };

    // Check for missing libraries
    for lib_path in &lib_paths {
        if !std::path::Path::new(lib_path).exists() {
            println!("Missing library: {}", lib_path);
        }
    }

    // Check for missing assets (virtual/legacy)
    let asset_index_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "minecraft/assets/indexes/{}.json",
            version_detail.assetIndex.id
        ))
        .expect("Failed to read asset index JSON"),
    )
    .expect("Failed to parse asset index JSON");
    if let Some(objects) = asset_index_json["objects"].as_object() {
        for (name, obj) in objects {
            let hash = obj["hash"].as_str().unwrap();
            let subdir = &hash[0..2];
            let virtual_path = format!("minecraft/assets/virtual/legacy/{}", name);
            if !std::path::Path::new(&virtual_path).exists() {
                println!("Missing asset: {}", virtual_path);
            }
        }
    }

    // Build classpath
    let mut classpath = lib_paths.join(";");
    classpath.push(';');
    classpath.push_str(&jar_path);

    // Prepare arguments 
    let main_class = &version_detail.mainClass;
    println!("Launching Minecraft with main class: {}", main_class);

    // Prepare minimal arguments for offline mode
    let username = args.username.unwrap_or_else(|| "Player".to_string());
    let args = vec![
        "--username",
        &username,
        "--version",
        selected_version,
        "--gameDir",
        "minecraft",
        "--assetsDir",
        "minecraft/assets",
        "--assetIndex",
        &version_detail.assetIndex.id,
        "--accessToken",
        "0",
        "--uuid",
        "0",
        "--userType",
        "msa",
        "--clientId",
        "0",
        "--xuid",
        "0",
    ];

    let mut command = std::process::Command::new("java");
    command.arg("-cp").arg(&classpath);
    command.arg(format!("-Djava.library.path=minecraft/natives"));
    command.arg(main_class);
    for arg in args {
        command.arg(arg);
    }

    let status = command.status();

    match status {
        Ok(s) if s.success() => println!("Minecraft launched successfully."),
        Ok(s) => println!("Minecraft exited with status: {}", s),
        Err(e) => println!("Failed to launch Minecraft: {}", e),
    }
}
