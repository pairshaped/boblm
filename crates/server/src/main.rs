use boblm_server::app::App;
use boblm_server::config;
use boblm_server::persistent::PersistentFactory;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
};
mod background_bash;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some(boblm_tuning::CLAUDE_HOOK_BACKGROUND_BASH) {
        std::process::exit(background_bash::run());
    }
    let runner = std::env::args().nth(1).as_deref() == Some("session-runner");
    let mut builder = if runner {
        tokio::runtime::Builder::new_current_thread()
    } else {
        tokio::runtime::Builder::new_multi_thread()
    };
    builder.enable_all().build()?.block_on(run())
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let [command, flag, path, session_flag, session] = args.as_slice()
        && command == "session-runner"
        && flag == "--config"
        && session_flag == "--session"
    {
        return boblm_server::persistent::serve(std::path::Path::new(path), session).await;
    }
    if let [command, flag, path] = args.as_slice()
        && command == "stop-runners"
        && flag == "--config"
    {
        return boblm_server::persistent::stop_all(std::path::Path::new(path)).await;
    }
    if args
        .first()
        .is_some_and(|command| command == "restart-runner")
    {
        let mut path = None;
        let mut session = None;
        let mut wait = boblm_tuning::RUNNER_RESTART_WAIT;
        let (options, remainder) = args[1..].as_chunks::<2>();
        for option in options {
            match option[0].as_str() {
                "--config" if path.is_none() => path = Some(PathBuf::from(&option[1])),
                "--session" if session.is_none() => session = Some(option[1].as_str()),
                "--wait" => wait = std::time::Duration::from_secs(option[1].parse::<u64>()
                    .map_err(|_| "The runner restart wait must be a whole number of seconds. Use --wait SECONDS.")?),
                _ => return Err("The runner restart options are invalid. Use boblm restart-runner --session ID [--wait SECONDS] [--config PATH].".into()),
            }
        }
        if !remainder.is_empty() {
            return Err("A runner restart option has no value. Use boblm restart-runner --session ID [--wait SECONDS] [--config PATH].".into());
        }
        let session = session.filter(|id| !id.is_empty()).ok_or(
            "Choose the chat to restart with --session ID. Other chats will be left running.",
        )?;
        let path = match path {
            Some(path) => path,
            None => config::default_path()?,
        };
        boblm_server::persistent::restart_runner(&path, session, wait).await?;
        println!(
            "Stopped this chat's quiet runner. BobLM will start its replacement when the web server reconnects."
        );
        return Ok(());
    }
    let (command, path) = match args.as_slice() {
        [command] if command == "serve" || command == "init" => (command, config::default_path()?),
        [command, flag, path]
            if (command == "serve" || command == "init") && flag == "--config" =>
        {
            (command, PathBuf::from(path))
        }
        _ => return Err("usage: boblm {init|serve} [--config PATH]".into()),
    };
    if command == "init" {
        let config = config::init(&path)?;
        let discovery = tokio::process::Command::new("tailscale")
            .args(["serve", "status", "--json"])
            .kill_on_drop(true)
            .output();
        let url = match tokio::time::timeout(boblm_tuning::TAILSCALE_DISCOVERY_TIMEOUT, discovery)
            .await
        {
            Ok(Ok(output)) if output.status.success() => serde_json::from_slice(&output.stdout)
                .ok()
                .and_then(|status| config::tailscale_url(&status, config.port)),
            _ => None,
        }
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", config.port));
        println!(
            "Created {}\nBearer token: {}\nURL: {url}\nKeep the listener on 127.0.0.1. Use Tailscale Serve HTTPS for other devices.",
            path.display(),
            config.token
        );
        return Ok(());
    }
    let config = config::read(&path)?;
    config::private_database(&config.database)?;
    let runtime_dir = config::runtime_dir(&config.database);
    config::owner_only_directory(&runtime_dir)?;
    let uploads_dir = boblm_server::uploads::uploads_dir(&config.database);
    config::owner_only_directory(&uploads_dir)?;
    let database = config.database.clone();
    let store = tokio::task::spawn_blocking(move || boblm_store::Store::open(database)).await??;
    let listener =
        tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port)).await?;
    let factory: Arc<dyn boblm_runner::DriverFactory> = if config.port == 0 {
        // Random-port instances cannot give surviving agents a stable MCP address.
        Arc::new(boblm_server::mcp::AgentFactory::new(
            format!("http://{}", listener.local_addr()?),
            config.token.clone(),
            runtime_dir,
            uploads_dir,
        ))
    } else {
        Arc::new(PersistentFactory::new(path.clone())?)
    };
    let app = App::open(store, factory, config.token).await?;
    if let Some(push) = config.notifications {
        app.configure_notifications(push)?;
    }
    app.sweep_orphan_uploads(boblm_tuning::ORPHAN_UPLOAD_AGE)
        .await
        .map_err(|_| "could not sweep abandoned uploads")?;
    app.start_scheduler();
    eprintln!("boblm listening on http://{}", listener.local_addr()?);
    let signal = async {
        #[cfg(unix)]
        {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut terminate) => {
                    tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = terminate.recv()=>{}}
                }
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    };
    let router = app.router(config.web_root);
    // Drop open SSE bodies on shutdown; graceful HTTP drain could wait forever.
    tokio::select! {
        result=axum::serve(listener,router)=>{result?;},
        _=signal=>{}
    }
    app.shutdown().await?;
    Ok(())
}
