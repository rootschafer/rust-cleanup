use std::{
	io::{self, Write},
	path::Path,
};

use clap::{Arg, ArgAction, Command};
use walkdir::WalkDir;

fn main() {
	let matches = Command::new("rust-cleanup")
		.arg(
			Arg::new("path")
				.short('p')
				.long("path")
				.value_name("PATH")
				.help("Sets the starting directory for the search"),
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

	let start_path = matches
		.get_one::<String>("path")
		.map_or(".", String::as_str);
	let yes_cargo = matches.get_flag("yes-cargo");
	let yes_dioxus = matches.get_flag("yes-dioxus");
	let yes_all = matches.get_flag("yes-all");


	let mut skipped_projects = Vec::new();

	for entry in WalkDir::new(start_path).into_iter().filter_map(|e| e.ok()) {
		if entry.file_type().is_dir() {
			let path = entry.path();

			let project_type = ProjectType::new_from_path(path);
			handle_project(
				path,
				&project_type,
				project_type.should_autoclean(yes_dioxus, yes_cargo, yes_all),
				&mut skipped_projects,
			);
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

#[derive(PartialEq, Clone)]
enum ProjectType {
	Regular,
	Rust,
	Dioxus,
}

impl ProjectType {
	fn new_from_path(path: &Path) -> Self {
		if path.join("Dioxus.toml").exists() {
			Self::Dioxus
		} else if path.join("Cargo.toml").exists() {
			Self::Rust
		} else {
			Self::Regular
		}
	}
	fn display_name(&self) -> &str {
		match self {
			Self::Regular => "Not a Rust project",
			Self::Rust => "Rust",
			Self::Dioxus => "Dioxus",
		}
	}

	fn clean_cmd(&self) -> &str {
		match self {
			Self::Regular => "",
			Self::Rust => "cargo clean",
			Self::Dioxus => "dx clean",
		}
	}

	fn should_autoclean(&self, yes_dioxus: bool, yes_cargo: bool, yes_all: bool) -> bool {
		match self {
			Self::Regular => false,
			Self::Rust => yes_cargo || yes_all,
			Self::Dioxus => yes_dioxus || yes_all,
		}
	}
}

fn handle_project(
	path: &Path,
	project_type: &ProjectType,
	auto_clean: bool,
	skipped_projects: &mut Vec<std::path::PathBuf>,
) {
	if *project_type == ProjectType::Regular {
		return;
	}

	if auto_clean || prompt_user(path, project_type) {
		// let status = std::process::Command::new(project_type.clean_cmd().split_whitespace().next().unwrap())
		//     .args(project_type.clean_cmd().split_whitespace().skip(1))
		//     .current_dir(path)
		//     .status()
		//     .expect("Clean command failed");
		//
		// if !status.success() {
		//     println!("There was an error cleaning {path:?}");
		// }

		match std::process::Command::new(project_type.clean_cmd().split_whitespace().next().unwrap())
			.args(project_type.clean_cmd().split_whitespace().skip(1))
			.current_dir(path)
			.status()
		{
			Ok(status) => {
				if !status.success() {
					println!("Command has a nonzero exit code while trying to clean {path:?}");
				}
			}
			Err(e) => {
				println!("There was an error cleaning {path:?}: {e}");
			}
		}
	} else {
		skipped_projects.push(path.to_path_buf());
	}
}

fn prompt_user(path: &std::path::Path, project_type: &ProjectType) -> bool {
	print!(
		"{} is a {} project. Do you want to clean it? (y/n): ",
		path.display(),
		project_type.display_name()
	);
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
			}
		}
	}
}
