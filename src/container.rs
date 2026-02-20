use crate::tracking;
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum ContainerCmd {
    DockerPs,
    DockerImages,
    DockerLogs,
    KubectlPods,
    KubectlServices,
    KubectlLogs,
}

pub fn run(cmd: ContainerCmd, args: &[String], verbose: u8) -> Result<()> {
    match cmd {
        ContainerCmd::DockerPs => docker_ps(verbose),
        ContainerCmd::DockerImages => docker_images(verbose),
        ContainerCmd::DockerLogs => docker_logs(args, verbose),
        ContainerCmd::KubectlPods => kubectl_pods(args, verbose),
        ContainerCmd::KubectlServices => kubectl_services(args, verbose),
        ContainerCmd::KubectlLogs => kubectl_logs(args, verbose),
    }
}

fn docker_ps(_verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let raw = Command::new("docker")
        .args(["ps"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let output = Command::new("docker")
        .args([
            "ps",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.Status}}\t{{.Image}}\t{{.Ports}}",
        ])
        .output()
        .context("Failed to run docker ps")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut rtk = String::new();

    if stdout.trim().is_empty() {
        rtk.push_str("🐳 0 containers");
        println!("{}", rtk);
        timer.track("docker ps", "rtk docker ps", &raw, &rtk);
        return Ok(());
    }

    let count = stdout.lines().count();
    rtk.push_str(&format!("🐳 {} containers:\n", count));

    for line in stdout.lines().take(15) {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 4 {
            let id = &parts[0][..12.min(parts[0].len())];
            let name = parts[1];
            let short_image = parts.get(3).unwrap_or(&"").split('/').last().unwrap_or("");
            let ports = compact_ports(parts.get(4).unwrap_or(&""));
            if ports == "-" {
                rtk.push_str(&format!("  {} {} ({})\n", id, name, short_image));
            } else {
                rtk.push_str(&format!(
                    "  {} {} ({}) [{}]\n",
                    id, name, short_image, ports
                ));
            }
        }
    }
    if count > 15 {
        rtk.push_str(&format!("  ... +{} more", count - 15));
    }

    print!("{}", rtk);
    timer.track("docker ps", "rtk docker ps", &raw, &rtk);
    Ok(())
}

fn docker_images(_verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let raw = Command::new("docker")
        .args(["images"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let output = Command::new("docker")
        .args(["images", "--format", "{{.Repository}}:{{.Tag}}\t{{.Size}}"])
        .output()
        .context("Failed to run docker images")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let mut rtk = String::new();

    if lines.is_empty() {
        rtk.push_str("🐳 0 images");
        println!("{}", rtk);
        timer.track("docker images", "rtk docker images", &raw, &rtk);
        return Ok(());
    }

    let mut total_size_mb: f64 = 0.0;
    for line in &lines {
        let parts: Vec<&str> = line.split('\t').collect();
        if let Some(size_str) = parts.get(1) {
            if size_str.contains("GB") {
                if let Ok(n) = size_str.replace("GB", "").trim().parse::<f64>() {
                    total_size_mb += n * 1024.0;
                }
            } else if size_str.contains("MB") {
                if let Ok(n) = size_str.replace("MB", "").trim().parse::<f64>() {
                    total_size_mb += n;
                }
            }
        }
    }

    let total_display = if total_size_mb > 1024.0 {
        format!("{:.1}GB", total_size_mb / 1024.0)
    } else {
        format!("{:.0}MB", total_size_mb)
    };
    rtk.push_str(&format!("🐳 {} images ({})\n", lines.len(), total_display));

    for line in lines.iter().take(15) {
        let parts: Vec<&str> = line.split('\t').collect();
        if !parts.is_empty() {
            let image = parts[0];
            let size = parts.get(1).unwrap_or(&"");
            let short = if image.len() > 40 {
                format!("...{}", &image[image.len() - 37..])
            } else {
                image.to_string()
            };
            rtk.push_str(&format!("  {} [{}]\n", short, size));
        }
    }
    if lines.len() > 15 {
        rtk.push_str(&format!("  ... +{} more", lines.len() - 15));
    }

    print!("{}", rtk);
    timer.track("docker images", "rtk docker images", &raw, &rtk);
    Ok(())
}

fn docker_logs(args: &[String], _verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let container = args.first().map(|s| s.as_str()).unwrap_or("");
    if container.is_empty() {
        println!("Usage: rtk docker logs <container>");
        return Ok(());
    }

    let output = Command::new("docker")
        .args(["logs", "--tail", "100", container])
        .output()
        .context("Failed to run docker logs")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    let analyzed = crate::log_cmd::run_stdin_str(&raw);
    let rtk = format!("🐳 Logs for {}:\n{}", container, analyzed);
    println!("{}", rtk);
    timer.track(
        &format!("docker logs {}", container),
        "rtk docker logs",
        &raw,
        &rtk,
    );
    Ok(())
}

fn kubectl_pods(args: &[String], _verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("kubectl");
    cmd.args(["get", "pods", "-o", "json"]);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run kubectl get pods")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let mut rtk = String::new();

    let json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            rtk.push_str("☸️  No pods found");
            println!("{}", rtk);
            timer.track("kubectl get pods", "rtk kubectl pods", &raw, &rtk);
            return Ok(());
        }
    };

    let items = json["items"].as_array();
    if items.is_none() || items.unwrap().is_empty() {
        rtk.push_str("☸️  No pods found");
        println!("{}", rtk);
        timer.track("kubectl get pods", "rtk kubectl pods", &raw, &rtk);
        return Ok(());
    }

    let pods = items.unwrap();
    let (mut running, mut pending, mut failed, mut restarts_total) = (0, 0, 0, 0i64);
    let mut issues: Vec<String> = Vec::new();

    for pod in pods {
        let ns = pod["metadata"]["namespace"].as_str().unwrap_or("-");
        let name = pod["metadata"]["name"].as_str().unwrap_or("-");
        let phase = pod["status"]["phase"].as_str().unwrap_or("Unknown");

        if let Some(containers) = pod["status"]["containerStatuses"].as_array() {
            for c in containers {
                restarts_total += c["restartCount"].as_i64().unwrap_or(0);
            }
        }

        match phase {
            "Running" => running += 1,
            "Pending" => {
                pending += 1;
                issues.push(format!("{}/{} Pending", ns, name));
            }
            "Failed" | "Error" => {
                failed += 1;
                issues.push(format!("{}/{} {}", ns, name, phase));
            }
            _ => {
                if let Some(containers) = pod["status"]["containerStatuses"].as_array() {
                    for c in containers {
                        if let Some(w) = c["state"]["waiting"]["reason"].as_str() {
                            if w.contains("CrashLoop") || w.contains("Error") {
                                failed += 1;
                                issues.push(format!("{}/{} {}", ns, name, w));
                            }
                        }
                    }
                }
            }
        }
    }

    let mut parts = Vec::new();
    if running > 0 {
        parts.push(format!("{} ✓", running));
    }
    if pending > 0 {
        parts.push(format!("{} pending", pending));
    }
    if failed > 0 {
        parts.push(format!("{} ✗", failed));
    }
    if restarts_total > 0 {
        parts.push(format!("{} restarts", restarts_total));
    }

    rtk.push_str(&format!("☸️  {} pods: {}\n", pods.len(), parts.join(", ")));
    if !issues.is_empty() {
        rtk.push_str("⚠️  Issues:\n");
        for issue in issues.iter().take(10) {
            rtk.push_str(&format!("  {}\n", issue));
        }
        if issues.len() > 10 {
            rtk.push_str(&format!("  ... +{} more", issues.len() - 10));
        }
    }

    print!("{}", rtk);
    timer.track("kubectl get pods", "rtk kubectl pods", &raw, &rtk);
    Ok(())
}

fn kubectl_services(args: &[String], _verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("kubectl");
    cmd.args(["get", "services", "-o", "json"]);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run kubectl get services")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let mut rtk = String::new();

    let json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            rtk.push_str("☸️  No services found");
            println!("{}", rtk);
            timer.track("kubectl get svc", "rtk kubectl svc", &raw, &rtk);
            return Ok(());
        }
    };

    let items = json["items"].as_array();
    if items.is_none() || items.unwrap().is_empty() {
        rtk.push_str("☸️  No services found");
        println!("{}", rtk);
        timer.track("kubectl get svc", "rtk kubectl svc", &raw, &rtk);
        return Ok(());
    }

    let services = items.unwrap();
    rtk.push_str(&format!("☸️  {} services:\n", services.len()));

    for svc in services.iter().take(15) {
        let ns = svc["metadata"]["namespace"].as_str().unwrap_or("-");
        let name = svc["metadata"]["name"].as_str().unwrap_or("-");
        let svc_type = svc["spec"]["type"].as_str().unwrap_or("-");
        let ports: Vec<String> = svc["spec"]["ports"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        let port = p["port"].as_i64().unwrap_or(0);
                        let target = p["targetPort"]
                            .as_i64()
                            .or_else(|| p["targetPort"].as_str().and_then(|s| s.parse().ok()))
                            .unwrap_or(port);
                        if port == target {
                            format!("{}", port)
                        } else {
                            format!("{}→{}", port, target)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        rtk.push_str(&format!(
            "  {}/{} {} [{}]\n",
            ns,
            name,
            svc_type,
            ports.join(",")
        ));
    }
    if services.len() > 15 {
        rtk.push_str(&format!("  ... +{} more", services.len() - 15));
    }

    print!("{}", rtk);
    timer.track("kubectl get svc", "rtk kubectl svc", &raw, &rtk);
    Ok(())
}

fn kubectl_logs(args: &[String], _verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let pod = args.first().map(|s| s.as_str()).unwrap_or("");
    if pod.is_empty() {
        println!("Usage: rtk kubectl logs <pod>");
        return Ok(());
    }

    let mut cmd = Command::new("kubectl");
    cmd.args(["logs", "--tail", "100", pod]);
    for arg in args.iter().skip(1) {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run kubectl logs")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let analyzed = crate::log_cmd::run_stdin_str(&raw);
    let rtk = format!("☸️  Logs for {}:\n{}", pod, analyzed);
    println!("{}", rtk);
    timer.track(
        &format!("kubectl logs {}", pod),
        "rtk kubectl logs",
        &raw,
        &rtk,
    );
    Ok(())
}

/// Format `docker compose ps --format` output into compact form.
/// Expects tab-separated lines: Name\tImage\tStatus\tPorts
/// (no header row — `--format` output is headerless)
pub fn format_compose_ps(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();

    if lines.is_empty() {
        return "🐳 0 compose services".to_string();
    }

    let mut result = format!("🐳 {} compose services:\n", lines.len());

    for line in lines.iter().take(20) {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 4 {
            let name = parts[0];
            let image = parts[1];
            let status = parts[2];
            let ports = parts[3];

            let short_image = image.split('/').next_back().unwrap_or(image);

            let port_str = if ports.trim().is_empty() {
                String::new()
            } else {
                let compact = compact_ports(ports.trim());
                if compact == "-" {
                    String::new()
                } else {
                    format!(" [{}]", compact)
                }
            };

            result.push_str(&format!(
                "  {} ({}) {}{}\n",
                name, short_image, status, port_str
            ));
        }
    }
    if lines.len() > 20 {
        result.push_str(&format!("  ... +{} more\n", lines.len() - 20));
    }

    result.trim_end().to_string()
}

/// Format `docker compose logs` output into compact form
pub fn format_compose_logs(raw: &str) -> String {
    if raw.trim().is_empty() {
        return "🐳 No logs".to_string();
    }

    // docker compose logs prefixes each line with "service-N  | "
    // Use the existing log deduplication engine
    let analyzed = crate::log_cmd::run_stdin_str(raw);
    format!("🐳 Compose logs:\n{}", analyzed)
}

/// Format `docker compose build` output into compact summary
pub fn format_compose_build(raw: &str) -> String {
    if raw.trim().is_empty() {
        return "🐳 Build: no output".to_string();
    }

    let mut result = String::new();

    // Extract the summary line: "[+] Building 12.3s (8/8) FINISHED"
    for line in raw.lines() {
        if line.contains("Building") && line.contains("FINISHED") {
            result.push_str(&format!("🐳 {}\n", line.trim()));
            break;
        }
    }

    if result.is_empty() {
        // No FINISHED line found — might still be building or errored
        if let Some(line) = raw.lines().find(|l| l.contains("Building")) {
            result.push_str(&format!("🐳 {}\n", line.trim()));
        } else {
            result.push_str("🐳 Build:\n");
        }
    }

    // Collect unique service names from build steps like "[web 1/4]"
    let mut services: Vec<String> = Vec::new();
    // find('[') returns byte offset — use byte slicing throughout
    // '[' and ']' are single-byte ASCII, so byte arithmetic is safe
    for line in raw.lines() {
        if let Some(start) = line.find('[') {
            if let Some(end) = line[start + 1..].find(']') {
                let bracket = &line[start + 1..start + 1 + end];
                let svc = bracket.split_whitespace().next().unwrap_or("");
                if !svc.is_empty() && svc != "+" && !services.contains(&svc.to_string()) {
                    services.push(svc.to_string());
                }
            }
        }
    }

    if !services.is_empty() {
        result.push_str(&format!("  Services: {}\n", services.join(", ")));
    }

    // Count build steps (lines starting with " => ")
    let step_count = raw
        .lines()
        .filter(|l| l.trim_start().starts_with("=> "))
        .count();
    if step_count > 0 {
        result.push_str(&format!("  Steps: {}", step_count));
    }

    result.trim_end().to_string()
}

fn compact_ports(ports: &str) -> String {
    if ports.is_empty() {
        return "-".to_string();
    }

    // Extract just the port numbers
    let port_nums: Vec<&str> = ports
        .split(',')
        .filter_map(|p| p.split("->").next().and_then(|s| s.split(':').last()))
        .collect();

    if port_nums.len() <= 3 {
        port_nums.join(", ")
    } else {
        format!(
            "{}, ... +{}",
            port_nums[..2].join(", "),
            port_nums.len() - 2
        )
    }
}

/// Runs an unsupported docker subcommand by passing it through directly
pub fn run_docker_passthrough(args: &[OsString], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("docker passthrough: {:?}", args);
    }
    let status = Command::new("docker")
        .args(args)
        .status()
        .context("Failed to run docker")?;

    let args_str = tracking::args_display(args);
    timer.track_passthrough(
        &format!("docker {}", args_str),
        &format!("rtk docker {} (passthrough)", args_str),
    );

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Run `docker compose ps` with compact output
pub fn run_compose_ps(verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    // Raw output for token tracking
    let raw_output = Command::new("docker")
        .args(["compose", "ps"])
        .output()
        .context("Failed to run docker compose ps")?;

    if !raw_output.status.success() {
        let stderr = String::from_utf8_lossy(&raw_output.stderr);
        eprintln!("{}", stderr);
        std::process::exit(raw_output.status.code().unwrap_or(1));
    }
    let raw = String::from_utf8_lossy(&raw_output.stdout).to_string();

    // Structured output for parsing (same pattern as docker_ps)
    let output = Command::new("docker")
        .args([
            "compose",
            "ps",
            "--format",
            "{{.Name}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}",
        ])
        .output()
        .context("Failed to run docker compose ps --format")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("{}", stderr);
        std::process::exit(output.status.code().unwrap_or(1));
    }
    let structured = String::from_utf8_lossy(&output.stdout).to_string();

    if verbose > 0 {
        eprintln!("raw docker compose ps:\n{}", raw);
    }

    let rtk = format_compose_ps(&structured);
    println!("{}", rtk);
    timer.track("docker compose ps", "rtk docker compose ps", &raw, &rtk);
    Ok(())
}

/// Run `docker compose logs` with deduplication
pub fn run_compose_logs(service: Option<&str>, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("docker");
    cmd.args(["compose", "logs", "--tail", "100"]);
    if let Some(svc) = service {
        cmd.arg(svc);
    }

    let output = cmd.output().context("Failed to run docker compose logs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("{}", stderr);
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    if verbose > 0 {
        eprintln!("raw docker compose logs:\n{}", raw);
    }

    let rtk = format_compose_logs(&raw);
    println!("{}", rtk);
    let svc_label = service.unwrap_or("all");
    timer.track(
        &format!("docker compose logs {}", svc_label),
        "rtk docker compose logs",
        &raw,
        &rtk,
    );
    Ok(())
}

/// Run `docker compose build` with summary output
pub fn run_compose_build(service: Option<&str>, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("docker");
    cmd.args(["compose", "build"]);
    if let Some(svc) = service {
        cmd.arg(svc);
    }

    let output = cmd.output().context("Failed to run docker compose build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("{}", stderr);
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    if verbose > 0 {
        eprintln!("raw docker compose build:\n{}", raw);
    }

    let rtk = format_compose_build(&raw);
    println!("{}", rtk);
    let svc_label = service.unwrap_or("all");
    timer.track(
        &format!("docker compose build {}", svc_label),
        "rtk docker compose build",
        &raw,
        &rtk,
    );
    Ok(())
}

/// Runs an unsupported docker compose subcommand by passing it through directly
pub fn run_compose_passthrough(args: &[OsString], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("docker compose passthrough: {:?}", args);
    }
    let status = Command::new("docker")
        .arg("compose")
        .args(args)
        .status()
        .context("Failed to run docker compose")?;

    let args_str = tracking::args_display(args);
    timer.track_passthrough(
        &format!("docker compose {}", args_str),
        &format!("rtk docker compose {} (passthrough)", args_str),
    );

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Dispatch `kubectl get <resource>` to specialized or generic filter
pub fn kubectl_get(resource: &str, args: &[String], verbose: u8) -> Result<()> {
    match resource {
        "pods" | "pod" | "po" => kubectl_pods(args, verbose),
        "services" | "service" | "svc" => kubectl_services(args, verbose),
        _ => kubectl_get_generic(resource, args, verbose),
    }
}

/// Generic `kubectl get <resource>` filter via JSON output
fn kubectl_get_generic(resource: &str, args: &[String], verbose: u8) -> Result<()> {
    // If user already specified -o/--output, fall back to passthrough
    if args.iter().any(|a| a == "-o" || a.starts_with("--output")) {
        let timer = tracking::TimedExecution::start();
        let mut cmd = Command::new("kubectl");
        cmd.arg("get").arg(resource);
        for arg in args {
            cmd.arg(arg);
        }
        let status = cmd.status().context("Failed to run kubectl")?;
        let label = format!("kubectl get {} {}", resource, args.join(" "));
        timer.track_passthrough(&label, &format!("rtk {} (passthrough)", label));
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return Ok(());
    }

    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("kubectl");
    cmd.args(["get", resource, "-o", "json"]);
    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("kubectl get {} -o json {}", resource, args.join(" "));
    }

    let output = cmd
        .output()
        .context(format!("Failed to run kubectl get {}", resource))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprint!("{}", stderr);
        let raw = format!("{}{}", String::from_utf8_lossy(&output.stdout), stderr);
        timer.track(
            &format!("kubectl get {}", resource),
            &format!("rtk kubectl get {}", resource),
            &raw,
            &stderr,
        );
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let rtk = filter_kubectl_get_json(&raw, resource);

    print!("{}", rtk);
    timer.track(
        &format!("kubectl get {}", resource),
        &format!("rtk kubectl get {}", resource),
        &raw,
        &rtk,
    );
    Ok(())
}

/// Pure filter: parse kubectl JSON and produce compact output
pub fn filter_kubectl_get_json(raw: &str, resource: &str) -> String {
    let json: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return format!("☸️  Failed to parse {} JSON", resource),
    };

    // Single resource (no "items" key) — kubectl get <resource> <name> -o json
    if json.get("items").is_none() {
        let ns = json["metadata"]["namespace"].as_str().unwrap_or_default();
        let name = json["metadata"]["name"].as_str().unwrap_or("-");
        let status = extract_resource_status(&json, resource);
        let label = if ns.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", ns, name)
        };
        return format!("☸️  {} {}: {}\n", resource, label, status);
    }

    let items = match json["items"].as_array() {
        Some(arr) => arr,
        None => return format!("☸️  No {} found\n", resource),
    };

    if items.is_empty() {
        return format!("☸️  No {} found\n", resource);
    }

    let mut result = format!("☸️  {} {}:\n", items.len(), resource);

    for item in items.iter().take(15) {
        let ns = item["metadata"]["namespace"].as_str().unwrap_or_default();
        let name = item["metadata"]["name"].as_str().unwrap_or("-");
        let status = extract_resource_status(item, resource);
        let label = if ns.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", ns, name)
        };
        result.push_str(&format!("  {} {}\n", label, status));
    }

    if items.len() > 15 {
        result.push_str(&format!("  +{} more\n", items.len() - 15));
    }

    result
}

/// Extract a compact status string per resource type
fn extract_resource_status(item: &serde_json::Value, resource: &str) -> String {
    match resource {
        "nodes" | "node" | "no" => extract_node_status(item),
        "deployments" | "deployment" | "deploy" => extract_deployment_status(item),
        "events" | "event" | "ev" => extract_event_status(item),
        "configmaps" | "configmap" | "cm" => extract_data_count(item, "configmap"),
        "secrets" | "secret" => extract_data_count(item, "secret"),
        "ingresses" | "ingress" | "ing" => extract_ingress_status(item),
        "jobs" | "job" => extract_job_status(item),
        "namespaces" | "namespace" | "ns" => extract_namespace_status(item),
        "daemonsets" | "daemonset" | "ds" => extract_daemonset_status(item),
        "statefulsets" | "statefulset" | "sts" => extract_statefulset_status(item),
        _ => extract_generic_status(item),
    }
}

fn extract_node_status(item: &serde_json::Value) -> String {
    let mut ready = false;
    if let Some(conditions) = item["status"]["conditions"].as_array() {
        for c in conditions {
            if c["type"].as_str() == Some("Ready") && c["status"].as_str() == Some("True") {
                ready = true;
            }
        }
    }
    let version = item["status"]["nodeInfo"]["kubeletVersion"]
        .as_str()
        .unwrap_or("-");
    let cpu = item["status"]["capacity"]["cpu"].as_str().unwrap_or("-");
    let mem = item["status"]["capacity"]["memory"].as_str().unwrap_or("-");
    let status = if ready { "Ready" } else { "NotReady" };
    format!(
        "{} {} cpu={} mem={}",
        status,
        version,
        cpu,
        compact_memory(mem)
    )
}

fn extract_deployment_status(item: &serde_json::Value) -> String {
    let replicas = item["status"]["replicas"].as_i64().unwrap_or(0);
    let ready = item["status"]["readyReplicas"].as_i64().unwrap_or(0);
    let available = item["status"]["availableReplicas"].as_i64().unwrap_or(0);
    format!("{}/{} ready, {} available", ready, replicas, available)
}

fn extract_event_status(item: &serde_json::Value) -> String {
    let reason = item["reason"].as_str().unwrap_or("-");
    let msg = item["message"].as_str().unwrap_or("");
    let truncated = if msg.len() > 80 { &msg[..80] } else { msg };
    let count = item["count"].as_i64().unwrap_or(1);
    if count > 1 {
        format!("{} (x{}) {}", reason, count, truncated)
    } else {
        format!("{} {}", reason, truncated)
    }
}

fn extract_data_count(item: &serde_json::Value, kind: &str) -> String {
    let count = item["data"].as_object().map(|o| o.len()).unwrap_or(0);
    format!("{} keys ({})", count, kind)
}

fn extract_ingress_status(item: &serde_json::Value) -> String {
    let rules: Vec<String> = item["spec"]["rules"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r["host"].as_str().map(|h| h.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if rules.is_empty() {
        "no rules".to_string()
    } else {
        rules.join(", ")
    }
}

fn extract_job_status(item: &serde_json::Value) -> String {
    let succeeded = item["status"]["succeeded"].as_i64().unwrap_or(0);
    let failed = item["status"]["failed"].as_i64().unwrap_or(0);
    let active = item["status"]["active"].as_i64().unwrap_or(0);
    if active > 0 {
        format!(
            "active={} succeeded={} failed={}",
            active, succeeded, failed
        )
    } else if failed > 0 {
        format!("Failed (succeeded={} failed={})", succeeded, failed)
    } else {
        format!("Complete ({})", succeeded)
    }
}

fn extract_namespace_status(item: &serde_json::Value) -> String {
    item["status"]["phase"].as_str().unwrap_or("-").to_string()
}

fn extract_daemonset_status(item: &serde_json::Value) -> String {
    let desired = item["status"]["desiredNumberScheduled"]
        .as_i64()
        .unwrap_or(0);
    let ready = item["status"]["numberReady"].as_i64().unwrap_or(0);
    format!("{}/{} ready", ready, desired)
}

fn extract_statefulset_status(item: &serde_json::Value) -> String {
    let replicas = item["status"]["replicas"].as_i64().unwrap_or(0);
    let ready = item["status"]["readyReplicas"].as_i64().unwrap_or(0);
    format!("{}/{} ready", ready, replicas)
}

fn extract_generic_status(item: &serde_json::Value) -> String {
    // Try common status paths
    if let Some(phase) = item["status"]["phase"].as_str() {
        return phase.to_string();
    }
    if let Some(conditions) = item["status"]["conditions"].as_array() {
        if let Some(last) = conditions.last() {
            let ctype = last["type"].as_str().unwrap_or("-");
            let status = last["status"].as_str().unwrap_or("-");
            return format!("{}={}", ctype, status);
        }
    }
    if let Some(ts) = item["metadata"]["creationTimestamp"].as_str() {
        return format!("created {}", ts);
    }
    "-".to_string()
}

/// Convert memory like "32901712Ki" to "31Gi"
fn compact_memory(mem: &str) -> String {
    if let Some(ki) = mem.strip_suffix("Ki") {
        if let Ok(n) = ki.parse::<u64>() {
            let gi = n / (1024 * 1024);
            if gi > 0 {
                return format!("{}Gi", gi);
            }
            let mi = n / 1024;
            return format!("{}Mi", mi);
        }
    }
    mem.to_string()
}

/// Runs an unsupported kubectl subcommand by passing it through directly
pub fn run_kubectl_passthrough(args: &[OsString], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("kubectl passthrough: {:?}", args);
    }
    let status = Command::new("kubectl")
        .args(args)
        .status()
        .context("Failed to run kubectl")?;

    let args_str = tracking::args_display(args);
    timer.track_passthrough(
        &format!("kubectl {}", args_str),
        &format!("rtk kubectl {} (passthrough)", args_str),
    );

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_compose_ps ──────────────────────────────────

    #[test]
    fn test_format_compose_ps_basic() {
        // Tab-separated --format output: Name\tImage\tStatus\tPorts
        let raw = "web-1\tnginx:latest\tUp 2 hours\t0.0.0.0:80->80/tcp\n\
                   api-1\tnode:20\tUp 2 hours\t0.0.0.0:3000->3000/tcp\n\
                   db-1\tpostgres:16\tUp 2 hours\t0.0.0.0:5432->5432/tcp";
        let out = format_compose_ps(raw);
        assert!(out.contains("3"), "should show container count");
        assert!(out.contains("web"), "should show service name");
        assert!(out.contains("api"), "should show service name");
        assert!(out.contains("db"), "should show service name");
        assert!(out.contains("Up 2 hours"), "should show status");
        assert!(out.len() < raw.len(), "output should be shorter than raw");
    }

    #[test]
    fn test_format_compose_ps_empty() {
        let out = format_compose_ps("");
        assert!(out.contains("0"), "should show zero containers");
    }

    #[test]
    fn test_format_compose_ps_whitespace_only() {
        let out = format_compose_ps("   \n  \n");
        assert!(out.contains("0"), "should show zero containers");
    }

    #[test]
    fn test_format_compose_ps_exited_service() {
        // Tab-separated --format output
        let raw = "worker-1\tpython:3.12\tExited (1) 2 minutes ago\t";
        let out = format_compose_ps(raw);
        assert!(out.contains("worker"), "should show service name");
        assert!(out.contains("Exited"), "should show exited status");
    }

    #[test]
    fn test_format_compose_ps_no_ports() {
        let raw = "redis-1\tredis:7\tUp 5 hours\t";
        let out = format_compose_ps(raw);
        assert!(out.contains("redis"), "should show service name");
        assert!(
            !out.contains("["),
            "should not show port brackets when empty"
        );
    }

    #[test]
    fn test_format_compose_ps_long_image_path() {
        let raw = "app-1\tghcr.io/myorg/myapp:latest\tUp 1 hour\t0.0.0.0:8080->8080/tcp";
        let out = format_compose_ps(raw);
        assert!(
            out.contains("myapp:latest"),
            "should shorten image to last segment"
        );
        assert!(
            !out.contains("ghcr.io"),
            "should not show full registry path"
        );
    }

    // ── format_compose_logs ────────────────────────────────

    #[test]
    fn test_format_compose_logs_basic() {
        let raw = "\
web-1  | 192.168.1.1 - GET / 200
web-1  | 192.168.1.1 - GET /favicon.ico 404
api-1  | Server listening on port 3000
api-1  | Connected to database";
        let out = format_compose_logs(raw);
        assert!(
            out.contains("Compose logs"),
            "should have compose logs header"
        );
    }

    #[test]
    fn test_format_compose_logs_empty() {
        let out = format_compose_logs("");
        assert!(out.contains("No logs"), "should indicate no logs");
    }

    // ── format_compose_build ───────────────────────────────

    #[test]
    fn test_format_compose_build_basic() {
        let raw = "\
[+] Building 12.3s (8/8) FINISHED
 => [web internal] load build definition from Dockerfile           0.0s
 => [web internal] load metadata for docker.io/library/node:20     1.2s
 => [web 1/4] FROM docker.io/library/node:20@sha256:abc123         0.0s
 => [web 2/4] WORKDIR /app                                         0.1s
 => [web 3/4] COPY package*.json ./                                0.1s
 => [web 4/4] RUN npm install                                      8.5s
 => [web] exporting to image                                       2.3s
 => => naming to docker.io/library/myapp-web                       0.0s";
        let out = format_compose_build(raw);
        assert!(out.contains("12.3s"), "should show total build time");
        assert!(out.contains("web"), "should show service name");
        assert!(out.len() < raw.len(), "should be shorter than raw");
    }

    #[test]
    fn test_format_compose_build_empty() {
        let out = format_compose_build("");
        assert!(
            !out.is_empty(),
            "should produce output even for empty input"
        );
    }

    // ── compact_ports (existing, previously untested) ──────

    #[test]
    fn test_compact_ports_empty() {
        assert_eq!(compact_ports(""), "-");
    }

    #[test]
    fn test_compact_ports_single() {
        let result = compact_ports("0.0.0.0:8080->80/tcp");
        assert!(result.contains("8080"));
    }

    #[test]
    fn test_compact_ports_many() {
        let result = compact_ports("0.0.0.0:80->80/tcp, 0.0.0.0:443->443/tcp, 0.0.0.0:8080->8080/tcp, 0.0.0.0:9090->9090/tcp");
        assert!(result.contains("..."), "should truncate for >3 ports");
    }

    // ── filter_kubectl_get_json ──────────────────────────────

    #[test]
    fn test_kubectl_get_json_empty_items() {
        let json = r#"{"apiVersion":"v1","kind":"List","items":[]}"#;
        let out = filter_kubectl_get_json(json, "configmaps");
        assert!(out.contains("No configmaps found"));
    }

    #[test]
    fn test_kubectl_get_json_invalid_json() {
        let out = filter_kubectl_get_json("not json at all", "pods");
        assert!(out.contains("Failed to parse"));
    }

    #[test]
    fn test_kubectl_get_json_single_resource() {
        let json = r#"{
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {"name": "my-config", "namespace": "default"},
            "data": {"key1": "val1", "key2": "val2"}
        }"#;
        let out = filter_kubectl_get_json(json, "configmaps");
        assert!(out.contains("default/my-config"));
        assert!(out.contains("2 keys"));
    }

    #[test]
    fn test_kubectl_get_json_deployments() {
        let json = r#"{"items": [{
            "metadata": {"name": "web", "namespace": "prod"},
            "status": {"replicas": 3, "readyReplicas": 3, "availableReplicas": 3}
        }, {
            "metadata": {"name": "api", "namespace": "prod"},
            "status": {"replicas": 2, "readyReplicas": 1, "availableReplicas": 1}
        }]}"#;
        let out = filter_kubectl_get_json(json, "deployments");
        assert!(out.contains("2 deployments"));
        assert!(out.contains("prod/web"));
        assert!(out.contains("3/3 ready"));
        assert!(out.contains("1/2 ready"));
    }

    #[test]
    fn test_kubectl_get_json_nodes() {
        let json = r#"{"items": [{
            "metadata": {"name": "node-1"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "nodeInfo": {"kubeletVersion": "v1.28.3"},
                "capacity": {"cpu": "8", "memory": "32901712Ki"}
            }
        }]}"#;
        let out = filter_kubectl_get_json(json, "nodes");
        assert!(out.contains("1 nodes"));
        assert!(out.contains("Ready"));
        assert!(out.contains("v1.28.3"));
        assert!(out.contains("cpu=8"));
        assert!(out.contains("31Gi"));
    }

    #[test]
    fn test_kubectl_get_json_events() {
        let json = r#"{"items": [{
            "metadata": {"name": "ev1", "namespace": "default"},
            "reason": "Pulled",
            "message": "Successfully pulled image nginx:latest",
            "count": 5
        }]}"#;
        let out = filter_kubectl_get_json(json, "events");
        assert!(out.contains("Pulled"));
        assert!(out.contains("x5"));
    }

    #[test]
    fn test_kubectl_get_json_truncation() {
        let mut items = String::from(r#"{"items": ["#);
        for i in 0..20 {
            if i > 0 {
                items.push(',');
            }
            items.push_str(&format!(
                r#"{{"metadata":{{"name":"cm-{}","namespace":"ns"}},"data":{{"k":"v"}}}}"#,
                i
            ));
        }
        items.push_str("]}");
        let out = filter_kubectl_get_json(&items, "configmaps");
        assert!(out.contains("20 configmaps"));
        assert!(out.contains("+5 more"));
    }

    #[test]
    fn test_kubectl_get_json_unknown_resource() {
        let json = r#"{"items": [{
            "metadata": {"name": "my-crd", "namespace": "default"},
            "status": {"phase": "Active"}
        }]}"#;
        let out = filter_kubectl_get_json(json, "mycustomresources");
        assert!(out.contains("1 mycustomresources"));
        assert!(out.contains("Active"));
    }

    #[test]
    fn test_kubectl_get_json_generic_conditions_fallback() {
        let json = r#"{"items": [{
            "metadata": {"name": "x", "namespace": "default"},
            "status": {"conditions": [{"type": "Available", "status": "True"}]}
        }]}"#;
        let out = filter_kubectl_get_json(json, "unknowns");
        assert!(out.contains("Available=True"));
    }

    #[test]
    fn test_kubectl_get_json_secrets() {
        let json = r#"{"items": [{
            "metadata": {"name": "db-creds", "namespace": "prod"},
            "data": {"username": "dXNlcg==", "password": "cGFzcw==", "host": "aG9zdA=="}
        }]}"#;
        let out = filter_kubectl_get_json(json, "secrets");
        assert!(out.contains("3 keys"));
        assert!(!out.contains("dXNlcg=="), "should not leak secret values");
    }

    #[test]
    fn test_kubectl_get_json_namespaces() {
        let json = r#"{"items": [
            {"metadata": {"name": "default"}, "status": {"phase": "Active"}},
            {"metadata": {"name": "kube-system"}, "status": {"phase": "Active"}}
        ]}"#;
        let out = filter_kubectl_get_json(json, "namespaces");
        assert!(out.contains("2 namespaces"));
        assert!(out.contains("Active"));
    }

    #[test]
    fn test_kubectl_get_json_jobs() {
        let json = r#"{"items": [{
            "metadata": {"name": "backup", "namespace": "default"},
            "status": {"succeeded": 1, "failed": 0, "active": 0}
        }]}"#;
        let out = filter_kubectl_get_json(json, "jobs");
        assert!(out.contains("Complete"));
    }

    #[test]
    fn test_kubectl_get_json_ingresses() {
        let json = r#"{"items": [{
            "metadata": {"name": "web-ing", "namespace": "default"},
            "spec": {"rules": [{"host": "app.example.com"}, {"host": "api.example.com"}]}
        }]}"#;
        let out = filter_kubectl_get_json(json, "ingresses");
        assert!(out.contains("app.example.com"));
        assert!(out.contains("api.example.com"));
    }

    #[test]
    fn test_kubectl_get_json_token_savings() {
        fn count_tokens(text: &str) -> usize {
            text.split_whitespace().count()
        }

        // Real kubectl output is pretty-printed JSON with lots of whitespace tokens
        let json = r#"{
    "apiVersion": "v1",
    "kind": "DeploymentList",
    "metadata": {
        "resourceVersion": "12345"
    },
    "items": [
        {
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "web",
                "namespace": "prod",
                "uid": "abc-123",
                "resourceVersion": "111",
                "generation": 5,
                "creationTimestamp": "2024-01-01T00:00:00Z",
                "labels": {
                    "app": "web",
                    "tier": "frontend"
                },
                "annotations": {
                    "deployment.kubernetes.io/revision": "3"
                }
            },
            "spec": {
                "replicas": 3,
                "selector": {
                    "matchLabels": {
                        "app": "web"
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": "web"
                        }
                    },
                    "spec": {
                        "containers": [
                            {
                                "name": "web",
                                "image": "nginx:1.25",
                                "ports": [
                                    {
                                        "containerPort": 80
                                    }
                                ],
                                "resources": {
                                    "requests": {
                                        "cpu": "100m",
                                        "memory": "128Mi"
                                    },
                                    "limits": {
                                        "cpu": "500m",
                                        "memory": "256Mi"
                                    }
                                }
                            }
                        ]
                    }
                }
            },
            "status": {
                "observedGeneration": 5,
                "replicas": 3,
                "updatedReplicas": 3,
                "readyReplicas": 3,
                "availableReplicas": 3,
                "conditions": [
                    {
                        "type": "Available",
                        "status": "True",
                        "lastUpdateTime": "2024-01-01T00:00:00Z",
                        "lastTransitionTime": "2024-01-01T00:00:00Z",
                        "reason": "MinimumReplicasAvailable",
                        "message": "Deployment has minimum availability."
                    },
                    {
                        "type": "Progressing",
                        "status": "True",
                        "lastUpdateTime": "2024-01-01T00:00:00Z",
                        "lastTransitionTime": "2024-01-01T00:00:00Z",
                        "reason": "NewReplicaSetAvailable",
                        "message": "ReplicaSet has successfully progressed."
                    }
                ]
            }
        },
        {
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "api",
                "namespace": "prod",
                "uid": "def-456",
                "resourceVersion": "222",
                "generation": 8,
                "creationTimestamp": "2024-01-02T00:00:00Z",
                "labels": {
                    "app": "api",
                    "tier": "backend"
                },
                "annotations": {
                    "deployment.kubernetes.io/revision": "8"
                }
            },
            "spec": {
                "replicas": 2,
                "selector": {
                    "matchLabels": {
                        "app": "api"
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": "api"
                        }
                    },
                    "spec": {
                        "containers": [
                            {
                                "name": "api",
                                "image": "node:20",
                                "ports": [
                                    {
                                        "containerPort": 3000
                                    }
                                ],
                                "resources": {
                                    "requests": {
                                        "cpu": "200m",
                                        "memory": "256Mi"
                                    },
                                    "limits": {
                                        "cpu": "1",
                                        "memory": "512Mi"
                                    }
                                }
                            }
                        ]
                    }
                }
            },
            "status": {
                "observedGeneration": 8,
                "replicas": 2,
                "updatedReplicas": 2,
                "readyReplicas": 2,
                "availableReplicas": 2,
                "conditions": [
                    {
                        "type": "Available",
                        "status": "True"
                    },
                    {
                        "type": "Progressing",
                        "status": "True"
                    }
                ]
            }
        }
    ]
}"#;
        let out = filter_kubectl_get_json(json, "deployments");

        let input_tokens = count_tokens(json);
        let output_tokens = count_tokens(&out);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 70.0,
            "Expected ≥70% savings, got {:.1}% (in={}, out={})",
            savings,
            input_tokens,
            output_tokens
        );
    }

    // ── compact_memory ───────────────────────────────────────

    #[test]
    fn test_compact_memory_gi() {
        assert_eq!(compact_memory("32901712Ki"), "31Gi");
    }

    #[test]
    fn test_compact_memory_mi() {
        assert_eq!(compact_memory("524288Ki"), "512Mi");
    }

    #[test]
    fn test_compact_memory_passthrough() {
        assert_eq!(compact_memory("4Gi"), "4Gi");
    }
}
