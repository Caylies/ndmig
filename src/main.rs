/*
    ndmig - A CLI tool for migrating Ballsdex to NationDex.
    By @Cayla
*/

use bollard::{
    Docker,
    exec::{CreateExecOptions, StartExecResults},
    query_parameters::ListContainersOptions,
};
use colored::*;
use futures_util::StreamExt;
use std::collections::HashMap;
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
/// #### Returns
///
/// Whether the container is classified as a Ballsdex instance.
///
async fn is_ballsdex_instance(docker: &Docker, container_id: &str) -> bool {
    let info = match docker.inspect_container(container_id, None).await {
        Ok(info) => info,
        Err(_) => return false,
    };

    info.config
        .and_then(|c| c.image)
        .map(|img| img == "postgres")
        .unwrap_or(false)
}

///
/// Creates a database dump by using `pg_dump` in the bot's postgres container.
///
/// #### Arguments
///
/// * `docker`: The Docker client.
/// * `container_id`: The container ID.
///
/// ### Returns
///
/// The SQL dump or an error.
///
async fn create_database_dump(docker: &Docker, container_id: &str) -> Result<String, bollard::errors::Error> {
    let info = docker.inspect_container(container_id, None).await?;
    let is_running = info.state.and_then(|s| s.running).unwrap_or(false);

    if !is_running {
        docker.start_container(container_id, None).await?;
    }

    let exec = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(vec!["pg_dump", "-U", "ballsdex"]), // Ballsdex database dump command thingy
                ..Default::default()
            },
        )
        .await?;

    let mut output = String::new();

    if let StartExecResults::Attached { output: mut stream, .. } = docker.start_exec(&exec.id, None).await? {
        while let Some(chunk) = stream.next().await {
            match chunk? {
                bollard::container::LogOutput::StdOut { message } => {
                    output.push_str(&String::from_utf8_lossy(&message));
                }
                bollard::container::LogOutput::StdErr { message } => {
                    eprintln!(
                        "{} {}",
                        "pg_dump stderr:".yellow().bold(),
                        String::from_utf8_lossy(&message)
                    );
                }
                _ => {}
            }
        }
    }

    Ok(output)
}

///
/// Formats a container name by removing the suffix.
///
/// #### Arguments
///
/// * `name`: The full container name.
///
/// #### Returns
///
/// The formatted name without the suffix.
///
fn format_name(name: &str) -> String {
    name.split("-").next().unwrap_or("unknown").to_string()
}

///
/// Displays the export selection menu and handles user input.
///
/// #### Arguments
///
/// * `instances`: A HashMap of Ballsdex instances names.
///
/// #### Returns
///
/// A tuple containing the instance name and its container ID.
///
fn export_setup(instances: &HashMap<String, String>) -> (String, String) {
    println!("\n{}", "Detected Ballsdex instances:".bold().yellow());

    for name in instances.keys() {
        println!("  {} {}", "›".bright_yellow(), format_name(name).bright_cyan());
    }

    print!("\n{}", "Select instance: ".bold().white());
    let _ = io::stdout().flush();

    let mut instance = String::new();
    io::stdin().read_line(&mut instance).expect("Failed to read input");

    let instance = instance.trim().to_string() + "-postgres-db-1";

    if !instances.contains_key(&instance) {
        clearscreen::clear().expect("Failed to clear screen");
        eprintln!(
            "{} {}",
            "✗ Instance not found:".red().bold(),
            format!("'{}'", instance).bright_red()
        );
        process::exit(1);
    }

    let container_id = instances[&instance].clone();
    (instance, container_id)
}

///
/// Starts the export setup process.
///
/// #### Arguments
///
/// * `docker`: The Docker client.
/// * `instances`: A HashMap of Ballsdex instances names.
///
async fn export(docker: &Docker, instances: &HashMap<String, String>) {
    let (instance, container_id) = export_setup(instances);

    let temp_dir = std::env::temp_dir().join("ndmig");
    std::fs::create_dir_all(&temp_dir).expect("Failed to create ndmig temp directory");

    let dump_path = temp_dir.join(format!("{}-ndmig.sql", container_id));

    println!("{}", "⧗ Exporting...".yellow().bold());

    match create_database_dump(docker, &container_id).await {
        Ok(sql) => {
            std::fs::write(&dump_path, sql).expect("Failed to create database dump");
            println!(
                "{}",
                format!("✓ {} has been successfully exported!", format_name(&instance),)
                    .green()
                    .bold()
            );
        }
        Err(e) => {
            eprintln!("{} {}", "✗ Export failed:".red().bold(), e);
            process::exit(1);
        }
    }
}

///
/// Promps the user to select an operation (export or import).
///
/// #### Arguments
///
/// * `docker`: The Docker client.
/// * `instances`: A HashMap of Ballsdex instances names.
///
async fn prompt(docker: &Docker, instances: &HashMap<String, String>) {
    println!(
        "{}",
        "Welcome to NDMIG, a Ballsdex to NationDex migration tool!\n"
            .bold()
            .bright_white()
    );

    println!("  1. Export"); // TODO: Make this look better
    println!("  2. Import");

    print!("\n{}", "Operation: ".bold().white());
    let _ = io::stdout().flush();

    let mut operation = String::new();
    io::stdin().read_line(&mut operation).expect("Failed to read input");

    match operation.trim() {
        "1" => export(docker, instances).await,
        "2" => println!("TBA"),
        _ => {
            eprintln!("{}", "✗ Invalid operation ('1' or '2').".red().bold());
            process::exit(1);
        }
    }
}

///
/// Main function for `ndmig`.
///
#[tokio::main]
async fn main() {
    clearscreen::clear().expect("Failed to clear screen");

    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(e) => {
            eprintln!("{} {}", "✗ Failed to connect to Docker:".red().bold(), e);
            process::exit(1);
        }
    };

    let options = Some(ListContainersOptions {
        all: true,
        ..Default::default()
    });

    let all = docker
        .list_containers(options)
        .await
        .expect("Failed to list containers");

    let mut instances: HashMap<String, String> = HashMap::new();

    for container in all {
        let id = match container.id.as_deref() {
            Some(id) => id,
            None => continue,
        };
        if is_ballsdex_instance(&docker, id).await {
            let project_name = container
                .names
                .iter()
                .flatten()
                .next()
                .map(|name| name.trim_start_matches("/").to_string())
                .unwrap_or_else(|| id.to_string());

            if project_name.ends_with("postgres-db-1") {
                instances.insert(project_name, id.to_string());
            }
        }
    }

    prompt(&docker, &instances).await;
}
