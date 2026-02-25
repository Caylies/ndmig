/*
    ndmig - A CLI tool for migrating Ballsdex to NationDex.
    By @Cayla
*/

use bollard::{Docker, query_parameters::ListContainersOptions};
use colored::*;
use std::collections::HashSet;
use std::io::{self, Write};
use std::process;

///
/// Checks if a Docker container is a Ballsdex instance by inspecting its image name.
///
/// #### Arguments
///
/// * `docker`: The Docker client.
/// * `container_id`: The ID of the container to inspect.
///
async fn is_ballsdex_instance(docker: &Docker, container_id: &str) -> bool {
    let info = match docker.inspect_container(container_id, None).await {
        Ok(info) => info,
        Err(_) => return false,
    };

    info.config
        .and_then(|c| c.image)
        .map(|img| img == "ballsdex")
        .unwrap_or(false)
}

///
/// Displays the project selection menu and handles user input.
///
/// #### Arguments
///
/// * `instances`: A set of detected Ballsdex instances names.
///
fn project_setup(instances: &HashSet<String>) {
    println!(
        "{}",
        "Welcome to NDMIG, a Ballsdex to NationDex migration tool!"
            .bold()
            .bright_white()
    );

    println!("\n{}", "Detected Ballsdex instances:".bold().yellow());

    for instance in instances {
        println!("  {} {}", "›".bright_yellow(), instance.bright_cyan());
    }

    print!("\n{}", "Select instance: ".bold().white());
    let _ = io::stdout().flush();

    let mut project = String::new();
    io::stdin().read_line(&mut project).expect("Failed to read input");

    let project = project.trim();

    if !instances.contains(project) {
        clearscreen::clear().expect("Failed to clear screen");
        eprintln!(
            "{} {}",
            "✗ Instance not found:".red().bold(),
            format!("'{}'", project).bright_red()
        );
        process::exit(1);
    }
}

///
/// Main function for `ndmig`.
///
#[tokio::main]
async fn main() {
    clearscreen::clear().expect("Failed to clear screen");

    let docker = Docker::connect_with_local_defaults().expect("Failed to connect to Docker");

    let options = Some(ListContainersOptions {
        all: true,
        ..Default::default()
    });

    let all = docker
        .list_containers(options)
        .await
        .expect("Failed to list containers");

    let mut containers = Vec::new();

    for container in all {
        let id = match container.id.as_deref() {
            Some(id) => id,
            None => continue,
        };

        if is_ballsdex_instance(&docker, id).await {
            containers.push(container);
        }
    }

    let projects: HashSet<String> = containers
        .iter()
        .flat_map(|c| c.names.iter().flatten())
        .map(|name| name.trim_start_matches("/").split("-").next().unwrap().to_string())
        .collect();

    project_setup(&projects);
}
