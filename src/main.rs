// use std::process::Command;
// use walkdir::WalkDir;
// use inquire::Confirm;
//
//
// fn main() {
//     let current_dir = std::env::current_dir().expect("Failed to get current directory");
//     let mut skipped_projects = Vec::new();
//
//     for entry in WalkDir::new(current_dir) {
//         let entry = entry.expect("Error accessing entry");
//         if entry.file_type().is_dir() {
//             let path = entry.path();
//             if path.join("Cargo.toml").exists() {
//                 handle_project(path, "Cargo.toml", "cargo clean", &mut skipped_projects);
//             } else if path.join("Dioxus.toml").exists() {
//                 handle_project(path, "Dioxus.toml", "dx clean", &mut skipped_projects);
//             }
//         }
//     }
//
//     if !skipped_projects.is_empty() {
//         println!("\nSkipped projects:");
//         for path in skipped_projects {
//             println!("{}", path.display());
//         }
//     }
// }
//
// fn handle_project(path: &std::path::Path, project_type_file: &str, clean_command: &str, skipped_projects: &mut Vec<std::path::PathBuf>) {
//     let message = if project_type_file == "Cargo.toml" {
//         format!("{} is a rust project. Do you want to clean it?", path.display())
//     } else {
//         format!("{} seems to be a Dioxus project. Do you want to clean it?", path.display())
//     };
//
//     let ans = Confirm::new(&message).prompt();
//
//     match ans {
//         Ok(true) => {
//             let status = Command::new(clean_command.split_whitespace().next().unwrap())
//                 .args(clean_command.split_whitespace().skip(1))
//                 .current_dir(path)
//                 .status();
//
//             if let Err(e) = status {
//                 eprintln!("Failed to run command: {}", e);
//             }
//         },
//         Ok(false) => skipped_projects.push(path.to_path_buf()),
//         Err(_) => {} // User pressed 'q' or Ctrl+C, do nothing
//     }
// }



use clap::{Arg, ArgAction, Command};
use walkdir::WalkDir;
use std::io::{self, Write};

fn main() {
    let matches = Command::new("rust-cleanup")
        .arg(
            Arg::new("path")
                .short('p')
                .long("path")
                .value_name("PATH")
                .help("Sets the starting directory for the search")
        )
        .arg(
            Arg::new("yes-cargo")
                .long("yes-cargo")
                .action(ArgAction::SetTrue)
                .help("Automatically clean non-Dioxus Rust projects without prompting"),
        )
        .arg(
            Arg::new("yes-dioxus")
                .long("yes-dioxus")
                .action(ArgAction::SetTrue)
                .help("Automatically clean Dioxus projects without prompting"),
        )
        .arg(
            Arg::new("yes-all")
                .long("yes-all")
                .short('y')
                .action(ArgAction::SetTrue)
                .help("Automatically clean all projects without prompting for a yes or a no"),
        )
        .get_matches();

    let start_path = matches.get_one::<String>("path").map_or(".", String::as_str);
    let yes_cargo = matches.get_flag("yes-cargo");
    let yes_dioxus = matches.get_flag("yes-dioxus");
    let yes_all = matches.get_flag("yes-all");


    let mut skipped_projects = Vec::new();

    for entry in WalkDir::new(start_path)
        .into_iter()
        .filter_map(|e| e.ok()) {
        if entry.file_type().is_dir() {
            let path = entry.path();
            if path.join("Cargo.toml").exists() {
                handle_project(path, "Rust", "cargo clean", yes_cargo || yes_all, &mut skipped_projects);
            } else if path.join("Dioxus.toml").exists() {
                handle_project(path, "Dioxus", "dx clean", yes_dioxus || yes_all, &mut skipped_projects);
            }
        }
    }

    // Print skipped projects
    if !skipped_projects.is_empty() {
        println!("Skipped projects:");
        for path in skipped_projects {
            println!("  {}", path.display());
        }
    }
}


fn handle_project(
    path: &std::path::Path, 
    project_type: &str, 
    clean_cmd: &str, 
    auto_clean: bool, 
    skipped_projects: &mut Vec<std::path::PathBuf>
) {
    if auto_clean || prompt_user(path, project_type) {
        let status = std::process::Command::new(clean_cmd.split_whitespace().next().unwrap())
            .args(clean_cmd.split_whitespace().skip(1))
            .current_dir(path)
            .status()
            .expect("Clean command failed");
        
        if !status.success() {
            println!("There was an error cleaning {path:?}");
        }
    } else {
        skipped_projects.push(path.to_path_buf());
    }
}

fn prompt_user(path: &std::path::Path, project_type: &str) -> bool {
    print!("{} is a {} project. Do you want to clean it? (y/n): ", path.display(), project_type);
    io::stdout().flush().unwrap();

    loop {
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        match input.trim().to_lowercase().as_str() {
            "y" => return true,
            "n" => return false,
            _ => {
                print!("Invalid input. Try one of (y/n): ");
                io::stdout().flush().unwrap();
            },     }
    }
}
