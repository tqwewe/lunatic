// wit_bindgen_host_wasmtime_rust::generate!({
//     name: "aggregate",
//     export: "aggregate.wit",
//     async: true,
// });

use anyhow::anyhow;

// Recursive expansion of generate! macro
// =======================================

#[allow(unused_imports)]
use wit_bindgen_host_wasmtime_rust::{anyhow, wasmtime};
pub type StateParam<'a> = &'a [u8];
pub type StateResult = Vec<u8>;
#[derive(wasmtime::component::ComponentType, wasmtime::component::Lower)]
#[component(record)]
#[derive(Clone)]
pub struct EventParam<'a> {
    #[component(name = "event-type")]
    pub event_type: &'a str,
    #[component(name = "payload")]
    pub payload: &'a [u8],
}
impl<'a> core::fmt::Debug for EventParam<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EventParam")
            .field("event-type", &self.event_type)
            .field("payload", &self.payload)
            .finish()
    }
}
#[derive(
    wasmtime::component::ComponentType, wasmtime::component::Lift, wasmtime::component::Lower,
)]
#[component(record)]
#[derive(Clone, Debug)]
pub struct EventResult {
    #[component(name = "event-type")]
    pub event_type: String,
    #[component(name = "payload")]
    pub payload: Vec<u8>,
}

#[derive(wasmtime::component::ComponentType, wasmtime::component::Lower)]
#[component(record)]
#[derive(Clone, Debug)]
pub struct Command<'a> {
    #[component(name = "command")]
    pub command: &'a str,
    #[component(name = "payload")]
    pub payload: &'a [u8],
}

#[derive(
    wasmtime::component::ComponentType, wasmtime::component::Lift, wasmtime::component::Lower,
)]
#[component(variant)]
#[derive(Clone, Debug)]
pub enum Error {
    #[component(name = "command")]
    Command(String),
    #[component(name = "custom")]
    Custom(String),
    #[component(name = "deserialize-command")]
    DeserializeCommand(String),
    #[component(name = "deserialize-event")]
    DeserializeEvent(String),
    #[component(name = "deserialize-state")]
    DeserializeState(String),
    #[component(name = "serialize-command")]
    SerializeCommand(String),
    #[component(name = "serialize-event")]
    SerializeEvent(String),
    #[component(name = "serialize-state")]
    SerializeState(String),
    #[component(name = "unknown-command")]
    UnknownCommand,
    #[component(name = "unknown-event")]
    UnknownEvent,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Command(msg) => write!(f, "command failed: {msg}"),
            Error::Custom(msg) => write!(f, "error: {msg}"),
            Error::DeserializeCommand(msg) => write!(f, "deserialize command failed: {msg}"),
            Error::DeserializeEvent(msg) => write!(f, "deserialize event failed: {msg}"),
            Error::DeserializeState(msg) => write!(f, "deserialize state failed: {msg}"),
            Error::SerializeCommand(msg) => write!(f, "serialize command failed: {msg}"),
            Error::SerializeEvent(msg) => write!(f, "serialize event failed: {msg}"),
            Error::SerializeState(msg) => write!(f, "serialize state failed: {msg}"),
            Error::UnknownCommand => write!(f, "unknown command"),
            Error::UnknownEvent => write!(f, "unknown event"),
        }
    }
}
impl std::error::Error for Error {}

impl From<Error> for wit_bindgen_host_wasmtime_rust::Error<Error> {
    fn from(e: Error) -> wit_bindgen_host_wasmtime_rust::Error<Error> {
        wit_bindgen_host_wasmtime_rust::Error::new(e)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Aggregate {
    init: wasmtime::component::Func,
    apply: wasmtime::component::Func,
    handle: wasmtime::component::Func,
}
impl Aggregate {
    pub fn new(
        exports: &mut wasmtime::component::ExportInstance<'_, '_>,
    ) -> anyhow::Result<Aggregate> {
        let init = *exports
            .typed_func::<(&str,), (Result<StateResult, Error>,)>("init")?
            .func();
        let apply = *exports
            .typed_func::<(StateParam<'_>, &[EventParam<'_>]), (Result<StateResult, Error>,)>(
                "apply",
            )?
            .func();
        let handle = *exports
            .typed_func::<(StateParam<'_>, Command<'_>), (Result<Vec<EventResult>, Error>,)>(
                "handle",
            )?
            .func();
        Ok(Aggregate {
            init,
            apply,
            handle,
        })
    }
    pub async fn init<S: wasmtime::AsContextMut>(
        &self,
        mut store: S,
        arg0: &str,
    ) -> anyhow::Result<Result<StateResult, Error>>
    where
        <S as wasmtime::AsContext>::Data: Send,
    {
        let callee = unsafe {
            wasmtime::component::TypedFunc::<(&str,), (Result<StateResult, Error>,)>::new_unchecked(
                self.init,
            )
        };
        let (ret0,) = callee.call_async(store.as_context_mut(), (arg0,)).await?;
        callee.post_return_async(store.as_context_mut()).await?;
        Ok(ret0)
    }
    pub async fn apply<S: wasmtime::AsContextMut>(
        &self,
        mut store: S,
        arg0: StateParam<'_>,
        arg1: &[EventParam<'_>],
    ) -> anyhow::Result<Result<StateResult, Error>>
    where
        <S as wasmtime::AsContext>::Data: Send,
    {
        let callee = unsafe {
            wasmtime::component::TypedFunc::<
                (StateParam<'_>, &[EventParam<'_>]),
                (Result<StateResult, Error>,),
            >::new_unchecked(self.apply)
        };
        let (ret0,) = callee
            .call_async(store.as_context_mut(), (arg0, arg1))
            .await?;
        callee.post_return_async(store.as_context_mut()).await?;
        Ok(ret0)
    }
    pub async fn handle<S: wasmtime::AsContextMut>(
        &self,
        mut store: S,
        arg0: StateParam<'_>,
        arg1: Command<'_>,
    ) -> anyhow::Result<Result<Vec<EventResult>, Error>>
    where
        <S as wasmtime::AsContext>::Data: Send,
    {
        let callee = unsafe {
            wasmtime::component::TypedFunc::<
                (StateParam<'_>, Command<'_>),
                (Result<Vec<EventResult>, Error>,),
            >::new_unchecked(self.handle)
        };
        let (ret0,) = callee
            .call_async(store.as_context_mut(), (arg0, arg1))
            .await?;
        callee.post_return_async(store.as_context_mut()).await?;
        Ok(ret0)
    }
}

/// Instantiates the provided `module` using the specified
/// parameters, wrapping up the result in a structure that
/// translates between wasm and the host.
pub async fn instantiate_async<T: Send>(
    mut store: impl wasmtime::AsContextMut<Data = T>,
    component: &wasmtime::component::Component,
    linker: &wasmtime::component::Linker<T>,
) -> anyhow::Result<(Aggregate, wasmtime::component::Instance)> {
    let instance = linker.instantiate_async(&mut store, component).await?;
    Ok((new(store, &instance)?, instance))
}

/// Low-level creation wrapper for wrapping up the exports
/// of the `instance` provided in this structure of wasm
/// exports.
///
/// This function will extract exports from the `instance`
/// defined within `store` and wrap them all up in the
/// returned structure which can be used to interact with
/// the wasm module.
pub fn new(
    mut store: impl wasmtime::AsContextMut,
    instance: &wasmtime::component::Instance,
) -> anyhow::Result<Aggregate> {
    let mut store = store.as_context_mut();
    let mut exports = instance.exports(&mut store);
    let mut exports = exports.root();
    let aggregate = Aggregate::new(
        &mut exports
            .instance("aggregate")
            .ok_or_else(|| anyhow!("no exported instance \"aggregate\""))?,
    )?;
    Ok(aggregate)
}

const _: &str = "";
