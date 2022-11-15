use anyhow::Result;
use lunatic_plugin_internal::PluginCtx;
use wasmtime::Linker;

pub use lunatic_runtime::DefaultProcessState;

pub trait Plugin: Sized {
    fn id() -> &'static str {
        std::any::type_name::<Self>()
    }

    fn init() -> Self;
    fn register(linker: &mut Linker<DefaultProcessState>) -> Result<()>;
}

pub trait LoadState {
    fn load_state<T>(&self) -> Option<&T>
    where
        T: Plugin + 'static;
    fn load_state_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Plugin + 'static;
}

impl LoadState for DefaultProcessState {
    fn load_state<T>(&self) -> Option<&T>
    where
        T: Plugin + 'static,
    {
        self.plugin_state(T::id())
    }

    fn load_state_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Plugin + 'static,
    {
        self.plugin_state_mut(T::id())
    }
}

#[macro_export]
macro_rules! register_plugin {
    ($plugin:ty) => {
        #[no_mangle]
        unsafe extern "C" fn plugin_id() -> &'static str {
            <$plugin as $crate::Plugin>::id()
        }

        #[no_mangle]
        unsafe extern "C" fn init() -> Box<dyn std::any::Any + Send + Sync> {
            Box::new(<$plugin as $crate::Plugin>::init())
        }

        #[no_mangle]
        unsafe extern "C" fn register(
            linker: &mut Linker<$crate::DefaultProcessState>,
        ) -> anyhow::Result<()> {
            <$plugin as $crate::Plugin>::register(linker)
        }
    };
}
