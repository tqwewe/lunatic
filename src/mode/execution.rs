use std::{collections::HashMap, env, fs, path::Path, sync::Arc};

use anyhow::{anyhow, Context, Ok, Result};
use clap::{crate_version, Arg, Command};
use tokio::sync::mpsc::channel;

use lunatic_distributed::{
    control::{self, server::control_server, Scanner, TokenType},
    distributed::{self, server::ServerCtx},
    quic,
};
use lunatic_process::{
    env::{Environments, LunaticEnvironments},
    runtimes::{self, Modules, RawWasm},
    wasm::spawn_wasm,
};
use lunatic_process_api::ProcessConfigCtx;
use lunatic_runtime::{DefaultProcessConfig, DefaultProcessState};

use uuid::Uuid;

/// Parse a single key-value pair
fn parse_key_val(s: &str) -> Result<(String, String)> {
    let scanner = Scanner::new(s.to_string());
    let tokens = scanner.scan()?;
    if tokens.len() == 3 {
        let key = &tokens[0];
        let equals = &tokens[1];
        let value = &tokens[2];
        if equals.t == TokenType::Equal {
            Ok((key.literal.clone(), value.literal.clone()))
        } else {
            Err(anyhow!("invalid key=value: no `=` found in `{}`", s))
        }
    } else {
        Err(anyhow!("invalid key=value syntax found in `{}`", s))
    }
}

pub(crate) async fn execute() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // Parse command line arguments
    let command = Command::new("lunatic")
        .version(crate_version!())
        .arg(
            Arg::new("dir")
                .long("dir")
                .value_name("DIRECTORY")
                .help("Grant access to the given host directory"),
        )
        .arg(
            Arg::new("node")
                .long("node")
                .value_name("NODE_ADDRESS")
                .help("Turns local process into a node and binds it to the provided address.")
                .requires("control"),
        )
        .arg(
            Arg::new("control")
                .long("control")
                .value_name("CONTROL_ADDRESS")
                .help("Address of a control node inside the cluster that will be used for bootstrapping.")
        )
        .arg(
            Arg::new("control_server")
                .long("control-server")
                .help("When set run the control server")
                .requires("control"),
        )
        .arg(
            Arg::new("test_ca")
                .long("test-ca")
                .help("Use test Certificate Authority for bootstrapping QUIC connections.")
                .requires("control")
        )
        .arg(
            Arg::new("ca_cert")
                .long("ca-cert")
                .help("Certificate Authority public certificate used for bootstrapping QUIC connections.")
                .requires("control")
                .conflicts_with("test_ca")
        )
        .arg(
            Arg::new("ca_key")
                .long("ca-key")
                .help("Certificate Authority private key used for signing node certificate requests")
                .requires("control_server")
                .conflicts_with("test_ca")
        )
        .arg(
            Arg::new("tag")
                .long("tag")
                .help("Define key=value variable to store as node information")
                .value_parser(parse_key_val)
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("no_entry")
                .long("no-entry")
                .help("If provided will join other nodes, but not require a .wasm entry file")
                .required_unless_present("wasm")
        ).arg(
            Arg::new("bench")
                .long("bench")
                .help("Indicate that a benchmark is running"),
        )
        .arg(
            Arg::new("wasm")
                .value_name("WASM")
                .help("Entry .wasm file")
                .conflicts_with("no_entry")
                .index(1),
        )
        .arg(
            Arg::new("wasm_args")
                .value_name("WASM_ARGS")
                .help("Arguments passed to the guest")
                .required(false)
                .conflicts_with("no_entry")
                .index(2),
        );

    #[cfg(feature = "prometheus")]
    let command = command
        .arg(
            Arg::new("prometheus")
                .long("prometheus")
                .help("whether to enable the prometheus metrics exporter"),
        )
        .arg(
            Arg::new("prometheus_http")
                .long("prometheus-http")
                .value_name("PROMETHEUS_HTTP_ADDRESS")
                .help("The address to bind the prometheus http listener to")
                .requires("prometheus")
                .takes_value(true)
                .default_value("0.0.0.0:9927"),
        );

    let args = command.get_matches();

    if args.get_flag("test_ca") {
        log::warn!("Do not use test Certificate Authority in production!")
    }

    // Run control server
    if args.get_flag("control_server") {
        if let Some(control_address) = args.get_one::<String>("control") {
            // TODO unwrap, better message
            let ca_cert = lunatic_distributed::control::server::root_cert(
                args.get_flag("test_ca"),
                args.get_one::<String>("ca_cert").map(|s| s.as_str()),
                args.get_one::<String>("ca_key").map(|s| s.as_str()),
            )
            .unwrap();
            tokio::task::spawn(control_server(control_address.parse().unwrap(), ca_cert));
        }
    }

    // Create wasmtime runtime
    let wasmtime_config = runtimes::wasmtime::default_config();
    let runtime = runtimes::wasmtime::WasmtimeRuntime::new(&wasmtime_config)?;
    let envs = Arc::new(LunaticEnvironments::default());

    let env = envs.create(1);

    let (distributed_state, control_client, node_id) =
        if let (Some(node_address), Some(control_address)) = (
            args.get_one::<String>("node"),
            args.get_one::<String>("control"),
        ) {
            // TODO unwrap, better message
            let node_address = node_address.parse().unwrap();
            let node_name = Uuid::new_v4().to_string();
            let node_attributes: HashMap<String, String> = args
                .get_many::<(String, String)>("tag")
                .map(|vals| vals.cloned().collect())
                .unwrap_or_default();
            let control_address = control_address.parse().unwrap();
            let ca_cert = lunatic_distributed::distributed::server::root_cert(
                args.get_flag("test_ca"),
                args.get_one::<String>("ca_cert").map(|s| s.as_str()),
            )
            .unwrap();
            let node_cert =
                lunatic_distributed::distributed::server::gen_node_cert(&node_name).unwrap();

            let quic_client = quic::new_quic_client(&ca_cert).unwrap();

            let (node_id, control_client, signed_cert_pem) = control::Client::register(
                node_address,
                node_name.to_string(),
                node_attributes,
                control_address,
                quic_client.clone(),
                node_cert.serialize_request_pem().unwrap(),
            )
            .await?;

            let distributed_client =
                distributed::Client::new(node_id, control_client.clone(), quic_client.clone())
                    .await?;

            let dist = lunatic_distributed::DistributedProcessState::new(
                node_id,
                control_client.clone(),
                distributed_client,
            )
            .await?;

            tokio::task::spawn(lunatic_distributed::distributed::server::node_server(
                ServerCtx {
                    envs,
                    modules: Modules::<DefaultProcessState>::default(),
                    distributed: dist.clone(),
                    runtime: runtime.clone(),
                },
                node_address,
                signed_cert_pem,
                node_cert.serialize_private_key_pem(),
            ));

            log::info!("Registration successful, node id {}", node_id);

            (Some(dist), Some(control_client), Some(node_id))
        } else {
            (None, None, None)
        };

    #[cfg(feature = "prometheus")]
    if args.get_flag("prometheus") {
        let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
        let builder = if let Some(addr) = args.get_one("prometheus_http") {
            builder.with_http_listener(addr.parse::<std::net::SocketAddr>().unwrap())
        } else {
            builder
        };

        let builder = if let Some(node_id) = node_id {
            builder.add_global_label("node_id", node_id.to_string())
        } else {
            builder
        };

        builder.install().unwrap()
    }

    let mut config = DefaultProcessConfig::default();
    // Allow initial process to compile modules, create configurations and spawn sub-processes
    config.set_can_compile_modules(true);
    config.set_can_create_configs(true);
    config.set_can_spawn_processes(true);

    if !args.get_flag("no_entry") {
        // Path to wasm file
        let path = args.get_one::<String>("wasm").unwrap();
        let path = Path::new(path);

        // Set correct command line arguments for the guest
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        let mut wasi_args = vec![filename];
        let wasm_args = args
            .get_many::<String>("wasm_args")
            .unwrap_or_default()
            .map(|s| s.to_string());
        wasi_args.extend(wasm_args);
        if args.get_flag("bench") {
            wasi_args.push("--bench".to_owned());
        }
        config.set_command_line_arguments(wasi_args);

        // Inherit environment variables
        config.set_environment_variables(env::vars().collect());

        // Always preopen the current dir
        config.preopen_dir(".");
        if let Some(dirs) = args.get_many::<String>("dir") {
            for dir in dirs {
                config.preopen_dir(dir);
            }
        }

        // Spawn main process
        let module = fs::read(path)?;
        let module: RawWasm = if let Some(dist) = distributed_state.as_ref() {
            dist.control.add_module(module).await?
        } else {
            module.into()
        };
        let module = Arc::new(runtime.compile_module::<DefaultProcessState>(module)?);
        let state = DefaultProcessState::new(
            env.clone(),
            distributed_state,
            runtime.clone(),
            module.clone(),
            Arc::new(config),
            Default::default(),
        )
        .unwrap();

        let (task, _) = spawn_wasm(env, runtime, &module, state, "_start", Vec::new(), None)
            .await
            .context(format!(
                "Failed to spawn process from {}::_start()",
                path.to_string_lossy()
            ))?;
        // Wait on the main process to finish
        let result = task.await.map(|_| ()).map_err(|e| anyhow!(e.to_string()));

        // Until we refactor registration and reconnect authentication, send node id explicitly
        if let (Some(ctrl), Some(node_id)) = (control_client, node_id) {
            ctrl.deregister(node_id).await;
        }

        result
    } else {
        // Block forever
        let (_sender, mut receiver) = channel::<()>(1);
        receiver.recv().await.unwrap();
        Ok(())
    }
}
