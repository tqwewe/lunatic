pub mod events_scratch;
pub mod module;
mod wit_aggregate;

use std::{fmt::Write, future::Future, io::Read};

use anyhow::Result;
use events_scratch::EventsScratch;
use hash_map_id::HashMapId;
use lunatic_common_api::{get_memory, IntoTrap};
use lunatic_process::state::ProcessState;
use lunatic_process_api::ProcessCtx;
use module::{Module, ModuleInstance};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use wasmtime::{Caller, Linker};

use crate::module::EventRef;

pub type AggregateModuleResources = HashMapId<Module>;
pub type AggregateModuleInstanceResources = HashMapId<(String, ModuleInstance)>;

pub trait AggregateModuleCtx {
    fn database_pool(&self) -> &Option<PgPool>;
    fn database_pool_mut(&mut self) -> &mut Option<PgPool>;
    fn aggregate_module_resources(&self) -> &AggregateModuleResources;
    fn aggregate_module_resources_mut(&mut self) -> &mut AggregateModuleResources;
    fn aggregate_module_instance_resources(&self) -> &AggregateModuleInstanceResources;
    fn aggregate_module_instance_resources_mut(&mut self) -> &mut AggregateModuleInstanceResources;
    fn events_scratch(&self) -> &Option<EventsScratch>;
    fn events_scratch_mut(&mut self) -> &mut Option<EventsScratch>;
}

// Register the mailbox APIs to the linker
pub fn register<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync + 'static>(
    linker: &mut Linker<T>,
) -> Result<()> {
    linker.func_wrap2_async("lunatic::thalo", "connect_db", connect_db)?;
    linker.func_wrap3_async("lunatic::thalo", "load_module", load_module)?;
    linker.func_wrap5_async("lunatic::thalo", "init_module", init_module)?;
    linker.func_wrap5_async("lunatic::thalo", "execute_command", execute_command)?;
    linker.func_wrap3_async("lunatic::thalo", "load_events", load_events)?;
    linker.func_wrap("lunatic::thalo", "read_events_data", read_events_data)?;
    linker.func_wrap2_async("lunatic::thalo", "stream_version", stream_version)?;

    Ok(())
}

// Connects to a database, initializing a pool.
//
// Traps:
// * If any memory outside the guest heap space is referenced.
// * The url is invalid utf8.
// * The module cannot be loaded.
fn connect_db<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync>(
    mut caller: Caller<T>,
    conn_ptr: u32,
    conn_len: u32,
) -> Box<dyn Future<Output = Result<u32>> + Send + '_> {
    Box::new(async move {
        let memory = get_memory(&mut caller)?;
        let conn_url_bytes = memory
            .data(&caller)
            .get(conn_ptr as usize..(conn_ptr as usize + conn_len as usize))
            .or_trap("lunatic::thalo::connect_db")?;
        let conn_url = std::str::from_utf8(conn_url_bytes).or_trap("lunatic::thalo::connect_db")?;
        let Ok(pool) = PgPool::connect(conn_url).await else {
            return Ok(1);
        };
        *caller.data_mut().database_pool_mut() = Some(pool);
        Ok(0)
    })
}

// Loads an aggregate module from a file name.
//
// Returns:
// * ID of newly created module in case of success.
// * -1 in case the module fails to load.
//
// Traps:
// * If any memory outside the guest heap space is referenced.
// * The file name is invalid utf8.
// * The module cannot be loaded.
fn load_module<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync>(
    mut caller: Caller<T>,
    fuel: u64,
    file_ptr: u32,
    file_len: u32,
) -> Box<dyn Future<Output = Result<i64>> + Send + '_> {
    Box::new(async move {
        let memory = get_memory(&mut caller)?;
        let file_name_bytes = memory
            .data(&caller)
            .get(file_ptr as usize..(file_ptr as usize + file_len as usize))
            .or_trap("lunatic::thalo::load_module")?;
        let file_name =
            std::str::from_utf8(file_name_bytes).or_trap("lunatic::thalo::load_module")?;
        let Ok(module) = Module::from_file(caller.engine().clone(), fuel, file_name).await else {
            return Ok(-1);
        };
        let index = caller
            .data_mut()
            .aggregate_module_resources_mut()
            .add(module);
        Ok(index as i64)
    })
}

// TODO
fn init_module<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync>(
    mut caller: Caller<T>,
    module: u64,
    stream_ptr: u32,
    stream_len: u32,
    id_ptr: u32,
    id_len: u32,
) -> Box<dyn Future<Output = Result<i64>> + Send + '_> {
    Box::new(async move {
        let memory = get_memory(&mut caller)?;

        // ID
        let id_bytes = memory
            .data(&caller)
            .get(id_ptr as usize..(id_ptr as usize + id_len as usize))
            .or_trap("lunatic::thalo::init_module")?;
        let id = std::str::from_utf8(id_bytes)
            .or_trap("lunatic::thalo::init_module")?
            .to_string();

        // Module
        let module = caller
            .data_mut()
            .aggregate_module_resources_mut()
            .get_mut(module)
            .or_trap("lunatic::thalo::init_module")?;

        // Initialize
        let mut instance = match module.init(id).await {
            Ok(instance) => instance,
            Err(err) => {
                println!("ERROR: {err}");
                return Ok(-1);
            }
        };
        println!("DONE initializing");

        // Stream
        let stream_bytes = memory
            .data(&caller)
            .get(stream_ptr as usize..(stream_ptr as usize + stream_len as usize))
            .or_trap("lunatic::thalo::init_module")?;
        let stream =
            String::from_utf8(stream_bytes.to_vec()).or_trap("lunatic::thalo::init_module")?;
        println!("DONE reading stream name from meory");

        let mut conn = caller
            .data()
            .database_pool()
            .as_ref()
            .or_trap("lunatic::thalo::init_module")?
            .acquire()
            .await
            .or_trap("lunatic::thalo::init_module")?;
        println!("DONE aquiring db connection");

        #[derive(Serialize, Deserialize)]
        struct Event {
            version: i64,
            r#type: String,
            body: Vec<u8>,
        }

        // Load events and update state
        let events = sqlx::query_as!(
            Event,
            "SELECT version, type, body FROM event WHERE stream = $1",
            stream
        )
        .fetch_all(&mut conn)
        .await
        .or_trap("lunatic::thalo::init_module")?;
        println!("DONE loading events from DB");

        let event_refs: Vec<_> = events
            .iter()
            .map(|event| EventRef {
                event_type: &event.r#type,
                payload: &event.body,
            })
            .collect();

        instance
            .apply(&event_refs)
            .await
            .or_trap("lunatic::thalo::init_module")?;
        println!("DONE applying events from DB");

        // Insert resource and return ID
        let index = caller
            .data_mut()
            .aggregate_module_instance_resources_mut()
            .add((stream, instance));
        Ok(index as i64)
    })
}

// TODO
#[allow(clippy::too_many_arguments)]
fn execute_command<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync>(
    mut caller: Caller<T>,
    command_ptr: u32,
    command_len: u32,
    payload_ptr: u32,
    payload_len: u32,
    instance: u64,
) -> Box<dyn Future<Output = Result<i64>> + Send + '_> {
    Box::new(async move {
        let memory = get_memory(&mut caller)?;

        // Command
        let command_bytes = memory
            .data(&caller)
            .get(command_ptr as usize..(command_ptr as usize + command_len as usize))
            .or_trap("lunatic::thalo::execute_command")?;
        let command =
            String::from_utf8(command_bytes.to_vec()).or_trap("lunatic::thalo::execute_command")?;
        println!("DONE reading command from meory");

        // Payload
        let payload = memory
            .data(&caller)
            .get(payload_ptr as usize..(payload_ptr as usize + payload_len as usize))
            .or_trap("lunatic::thalo::execute_command")?
            .to_vec();
        println!("DONE reading payload from meory");

        // Instance
        let (stream, instance) = caller
            .data_mut()
            .aggregate_module_instance_resources_mut()
            .get_mut(instance)
            .or_trap("lunatic::thalo::execute_command")?;
        let stream = stream.clone();
        println!("DONE reading stream and instance from id");

        let result = instance
            .handle_and_apply(&command, &payload)
            .await
            .map_err(|err| err.to_string());
        let events = match result {
            Ok(events) => events,
            Err(err) => {
                println!("Failed to handle and apply command: {err}");
                let buffer = bincode::serialize(&err).or_trap("lunatic::thalo::execute_command")?;
                caller
                    .data_mut()
                    .events_scratch_mut()
                    .replace(EventsScratch::new(buffer));
                return Ok(-1);
            }
        };
        println!("DONE handling and applying command");

        println!("FUEL COSNUMED: {}", instance.fuel_consumed().await);

        let events_count = events.len();
        if events_count == 0 {
            return Ok(0);
        }

        let mut conn = caller
            .data()
            .database_pool()
            .as_ref()
            .or_trap("lunatic::thalo::execute_command")?
            .acquire()
            .await
            .or_trap("lunatic::thalo::execute_command")?;
        println!("DONE aquiring db connection");

        let version = sqlx::query_scalar!(
            "SELECT MAX(version) as version FROM event WHERE stream = $1",
            &&*stream
        )
        .fetch_one(&mut conn)
        .await
        .or_trap("lunatic::thalo::execute_command")?
        .unwrap_or(-1);
        println!("DONE fetching latest version from db");

        let mut query = "INSERT INTO event (
            stream,
            version,
            type,
            body
        ) VALUES "
            .to_string();
        for i in 0..events.len() {
            write!(
                query,
                "(${}, ${}, ${}, ${})",
                (i * 4) + 1,
                (i * 4) + 2,
                (i * 4) + 3,
                (i * 4) + 4,
            )?;
        }

        let query = events
            .into_iter()
            .fold(
                (sqlx::query(&query), version),
                |(query, mut version), event| {
                    version += 1;
                    (
                        query
                            .bind(&stream)
                            .bind(version)
                            .bind(event.event_type)
                            .bind(event.payload),
                        version,
                    )
                },
            )
            .0;
        query
            .execute(&mut conn)
            .await
            .or_trap("lunatic::thalo::execute_command")?;
        println!("DONE saving new events into database");

        Ok(events_count as i64)
    })
}

// Loads events for a given stream.
//
// Returns:
// * 0 for success
// * 1 for error
//
// Traps:
// * If any memory outside the guest heap space is referenced.
// * The stream name is invalid utf8.
// * The database connection has not been established.
// * The database connection could not be aquired from the pool.
// * The database query fails.
fn load_events<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync>(
    mut caller: Caller<T>,
    stream_ptr: u32,
    stream_len: u32,
    from_version: i64,
) -> Box<dyn Future<Output = Result<()>> + Send + '_> {
    Box::new(async move {
        let memory = get_memory(&mut caller)?;
        let stream_bytes = memory
            .data(&caller)
            .get(stream_ptr as usize..(stream_ptr as usize + stream_len as usize))
            .or_trap("lunatic::thalo::load_events")?;
        let stream = std::str::from_utf8(stream_bytes).or_trap("lunatic::thalo::load_events")?;
        let mut conn = caller
            .data()
            .database_pool()
            .as_ref()
            .or_trap("lunatic::thalo::load_events")?
            .acquire()
            .await
            .or_trap("lunatic::thalo::load_events")?;
        #[derive(Serialize, Deserialize)]
        struct Event {
            version: i64,
            r#type: String,
            body: Vec<u8>,
        }
        let events = if from_version < 0 {
            sqlx::query_as!(
                Event,
                "SELECT version, type, body FROM event WHERE stream = $1",
                stream
            )
            .fetch_all(&mut conn)
            .await
        } else {
            sqlx::query_as!(
                Event,
                "SELECT version, type, body FROM event WHERE stream = $1",
                stream
            )
            .fetch_all(&mut conn)
            .await
        }
        .or_trap("lunatic::thalo::load_events")?;

        let events_data = bincode::serialize(&events).or_trap("lunatic::thalo::load_events")?;
        caller
            .data_mut()
            .events_scratch_mut()
            .replace(EventsScratch::new(events_data));

        Ok(())
    })
}

// Reads some data from the message buffer and returns how much data is read in bytes.
//
// Traps:
// * If any memory outside the guest heap space is referenced.
// * If it's called without a data message being inside of the scratch area.
fn read_events_data<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx>(
    mut caller: Caller<T>,
    data_ptr: u32,
    data_len: u32,
) -> Result<u32> {
    let memory = get_memory(&mut caller)?;
    let mut events_scratch = caller
        .data_mut()
        .events_scratch_mut()
        .take()
        .or_trap("lunatic::message::read_events_data")?;
    let buffer = memory
        .data_mut(&mut caller)
        .get_mut(data_ptr as usize..(data_ptr as usize + data_len as usize))
        .or_trap("lunatic::message::read_events_data")?;
    let bytes = events_scratch
        .read(buffer)
        .or_trap("lunatic::message::read_data")?;
    // Put message back after reading from it.
    caller
        .data_mut()
        .events_scratch_mut()
        .replace(events_scratch);

    Ok(bytes as u32)
}

// Loads the latest version of a stream.
//
// Returns:
// * Latest version of stream.
// * -1 in case the stream has no events.
//
// Traps:
// * If any memory outside the guest heap space is referenced.
// * The stream name is invalid utf8.
// * The database connection has not been established.
// * The database connection could not be aquired from the pool.
// * The database query fails.
fn stream_version<T: ProcessState + ProcessCtx<T> + AggregateModuleCtx + Send + Sync>(
    mut caller: Caller<T>,
    stream_ptr: u32,
    stream_len: u32,
) -> Box<dyn Future<Output = Result<i64>> + Send + '_> {
    Box::new(async move {
        let memory = get_memory(&mut caller)?;
        let stream_bytes = memory
            .data(&caller)
            .get(stream_ptr as usize..(stream_ptr as usize + stream_len as usize))
            .or_trap("lunatic::thalo::stream_version")?;
        let stream = std::str::from_utf8(stream_bytes).or_trap("lunatic::thalo::stream_version")?;
        let mut conn = caller
            .data()
            .database_pool()
            .as_ref()
            .or_trap("lunatic::thalo::stream_version")?
            .acquire()
            .await
            .or_trap("lunatic::thalo::stream_version")?;
        let version = sqlx::query_scalar!(
            "SELECT MAX(version) as version FROM event WHERE stream = $1",
            stream
        )
        .fetch_one(&mut conn)
        .await
        .or_trap("lunatic::thalo::stream_version")?;

        Ok(version.unwrap_or(-1))
    })
}
