use std::{borrow, fmt, ops::DerefMut, path::Path, str, sync::Arc};

use anyhow::{anyhow, Result};
use derive_more::{Deref, DerefMut};
use lunatic_process::config::UNIT_OF_COMPUTE_IN_INSTRUCTIONS;
use once_cell::sync::OnceCell;
use regex::Regex;
use semver::Version;
use serde::Serialize;
use tokio::sync::Mutex;
use wasmtime::{
    component::{Component, Linker},
    Engine, Store,
};

use crate::wit_aggregate::{self, Aggregate, Command};

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleID {
    pub name: ModuleName,
    pub version: Version,
}

#[derive(Clone, Debug, Deref, DerefMut, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleName(String);

#[derive(Clone)]
pub struct Module {
    aggregate: Aggregate,
    // component: Component,
    // engine: Engine,
    // instance: Instance,
    // linker: Linker<()>,
    store: Arc<Mutex<Store<()>>>,
}

pub struct ModuleInstance {
    aggregate: Aggregate,
    id: String,
    pub state: Vec<u8>,
    store: Arc<Mutex<Store<()>>>,
}

#[derive(Serialize)]
pub struct Event {
    pub event_type: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct EventRef<'a> {
    pub event_type: &'a str,
    pub payload: &'a [u8],
}

impl ModuleID {
    pub fn new(name: ModuleName, version: Version) -> Self {
        ModuleID { name, version }
    }
}

impl ModuleName {
    pub fn new<T>(name: T) -> Result<Self>
    where
        T: AsRef<str> + Into<String>,
    {
        static MODULE_NAME_REGEX: OnceCell<Regex> = OnceCell::new();
        let module_name_regex =
            MODULE_NAME_REGEX.get_or_init(|| Regex::new("^[a-z](?:\\-?[a-z])*$").unwrap());

        if !module_name_regex.is_match(name.as_ref()) {
            return Err(anyhow!(
                "module names must match the regex of ^[a-z](?:\\-?[a-z])*$"
            ));
        }

        Ok(ModuleName(name.into()))
    }
}

impl str::FromStr for ModuleName {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        ModuleName::new(name)
    }
}

impl TryFrom<String> for ModuleName {
    type Error = anyhow::Error;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        ModuleName::new(name)
    }
}

impl TryFrom<&str> for ModuleName {
    type Error = anyhow::Error;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        ModuleName::new(name.to_string())
    }
}

impl borrow::Borrow<str> for ModuleName {
    fn borrow(&self) -> &str {
        self.0.borrow()
    }
}

impl fmt::Display for ModuleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Module {
    pub async fn from_file(engine: Engine, fuel: u64, file: impl AsRef<Path>) -> Result<Self> {
        let mut store = Store::new(&engine, ());
        store.out_of_fuel_trap();
        store.out_of_fuel_async_yield(fuel, UNIT_OF_COMPUTE_IN_INSTRUCTIONS);
        let component = Component::from_file(&engine, file).unwrap();
        let linker = Linker::new(&engine);

        let (aggregate, _instance) =
            wit_aggregate::instantiate_async(&mut store, &component, &linker).await?;

        Ok(Module {
            aggregate,
            // component,
            // engine,
            // instance,
            // linker,
            store: Arc::new(Mutex::new(store)),
        })
    }

    pub async fn init<T>(&mut self, id: T) -> Result<ModuleInstance>
    where
        T: Into<String>,
    {
        let id = id.into();
        let aggregate = self.aggregate;
        let state = {
            let mut store = self.store.lock().await;
            aggregate.init(store.deref_mut(), &id).await??
        };

        Ok(ModuleInstance {
            aggregate,
            id,
            state,
            store: Arc::clone(&self.store),
        })
    }

    pub async fn fuel_consumed(&self) -> u64 {
        self.store.lock().await.fuel_consumed().unwrap()
    }
}

impl ModuleInstance {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn apply(&mut self, events: &[EventRef<'_>]) -> Result<()> {
        let events: Vec<_> = events.iter().map(|event| (*event).into()).collect();

        self.state = {
            let mut store = self.store.lock().await;
            self.aggregate
                .apply(store.deref_mut(), &self.state, &events)
                .await??
        };

        Ok(())
    }

    pub async fn handle(&mut self, command: &str, payload: &[u8]) -> Result<Vec<Event>> {
        let command = Command { command, payload };
        let events = {
            let mut store = self.store.lock().await;
            self.aggregate
                .handle(store.deref_mut(), &self.state, command)
                .await?
                .map_err(|err| anyhow!(err))?
                .into_iter()
                .map(Event::from)
                .collect()
        };

        Ok(events)
    }

    pub async fn handle_and_apply(&mut self, command: &str, payload: &[u8]) -> Result<Vec<Event>> {
        let events = self.handle(command, payload).await?;
        let event_refs: Vec<_> = events.iter().map(Event::as_ref).collect();
        self.apply(&event_refs).await?;
        Ok(events)
    }

    pub async fn fuel_consumed(&self) -> u64 {
        self.store.lock().await.fuel_consumed().unwrap()
    }
}

impl Event {
    pub fn as_ref(&self) -> EventRef {
        EventRef {
            event_type: &self.event_type,
            payload: &self.payload,
        }
    }
}

impl From<wit_aggregate::EventResult> for Event {
    fn from(event: wit_aggregate::EventResult) -> Self {
        Event {
            event_type: event.event_type,
            payload: event.payload,
        }
    }
}

impl<'a> From<EventRef<'a>> for wit_aggregate::EventParam<'a> {
    fn from(event: EventRef<'a>) -> Self {
        wit_aggregate::EventParam {
            event_type: event.event_type,
            payload: event.payload,
        }
    }
}
