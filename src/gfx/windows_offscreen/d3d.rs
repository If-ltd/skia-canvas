use skia_safe::gpu::{self, DirectContext, d3d};
use windows::Win32::Graphics::{
    Direct3D::D3D_FEATURE_LEVEL_11_0,
    Direct3D12::{D3D12_COMMAND_QUEUE_DESC, D3D12CreateDevice, ID3D12Device},
    Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIAdapter1,
        IDXGIFactory1,
    },
};

pub(super) struct Context {
    pub(super) direct: DirectContext,
    // Drop the Skia context before the COM objects it uses.
    backend: d3d::BackendContext,
    pub(super) device: String,
}

impl Context {
    pub(super) fn new() -> Result<Self, String> {
        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.map_err(|err| err.to_string())?;
        let mut failure = "no suitable hardware adapter".to_string();
        for index in 0.. {
            let adapter = match unsafe { factory.EnumAdapters1(index) } {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(error.to_string()),
            };
            let description = unsafe { adapter.GetDesc1() }.map_err(|err| err.to_string())?;
            if description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let name_len = description
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(description.Description.len());
            let name = String::from_utf16_lossy(&description.Description[..name_len]);
            match Self::for_adapter(adapter, name.clone()) {
                Ok(context) => return Ok(context),
                Err(error) => failure = format!("{name}: {error}"),
            }
        }
        Err(failure)
    }

    fn for_adapter(adapter: IDXGIAdapter1, name: String) -> Result<Self, String> {
        let mut device: Option<ID3D12Device> = None;
        unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) }
            .map_err(|err| err.to_string())?;
        let device = device.ok_or("D3D12CreateDevice returned no device")?;
        // Default queue descriptor is a direct graphics queue at normal priority.
        let queue = unsafe { device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default()) }
            .map_err(|err| err.to_string())?;
        let backend = d3d::BackendContext {
            adapter,
            device,
            queue,
            memory_allocator: None,
            protected_context: gpu::Protected::No,
        };
        let direct = unsafe { gpu::direct_contexts::make_d3d(&backend, None) }
            .ok_or("could not create Skia context")?;
        Ok(Self {
            direct,
            backend,
            device: name,
        })
    }

    pub(super) fn device_lost(&self) -> bool {
        unsafe { self.backend.device.GetDeviceRemovedReason().is_err() }
    }
}
