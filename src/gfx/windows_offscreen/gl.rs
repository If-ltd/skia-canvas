use std::ffi::CStr;

use skia_safe::gpu::{self, DirectContext, gl};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND},
        Graphics::{
            Gdi::{GetDC, HDC, ReleaseDC},
            OpenGL::{
                ChoosePixelFormat, GL_RENDERER, HGLRC, PFD_DRAW_TO_WINDOW, PFD_MAIN_PLANE,
                PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat,
                glGetString, wglCreateContext, wglDeleteContext, wglMakeCurrent,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP},
    },
    core::w,
};

pub(super) struct Context {
    pub(super) direct: DirectContext,
    // Skia must release GL resources while the WGL context is current.
    _wgl: WglContext,
    pub(super) device: String,
}

impl Context {
    pub(super) fn new() -> Result<Self, String> {
        let wgl = WglContext::new()?;
        let interface = gl::interfaces::make_win().ok_or("could not load WGL interface")?;
        let direct = gpu::direct_contexts::make_gl(interface, None)
            .ok_or("could not create Skia context")?;
        let renderer = unsafe { glGetString(GL_RENDERER) };
        let device = if renderer.is_null() {
            "OpenGL renderer".into()
        } else {
            unsafe { CStr::from_ptr(renderer.cast()) }
                .to_string_lossy()
                .into_owned()
        };
        Ok(Self {
            direct,
            _wgl: wgl,
            device,
        })
    }
}

// WGL needs a Win32 drawable to make a context current. It stays hidden and is
// used only for context binding. Skia draws into its own offscreen render targets.
struct WglContext {
    window: HWND,
    dc: HDC,
    context: HGLRC,
}

impl WglContext {
    fn new() -> Result<Self, String> {
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|err| err.to_string())?;
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                None,
            )
        }
        .map_err(|err| err.to_string())?;

        let mut wgl = Self {
            window,
            dc: unsafe { GetDC(Some(window)) },
            context: HGLRC::default(),
        };
        if wgl.dc.is_invalid() {
            return Err("could not obtain WGL drawable".into());
        }
        let format = PIXELFORMATDESCRIPTOR {
            nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            iLayerType: PFD_MAIN_PLANE.0 as u8,
            ..Default::default()
        };
        let index = unsafe { ChoosePixelFormat(wgl.dc, &format) };
        if index == 0 {
            return Err("no compatible WGL pixel format".into());
        }
        unsafe { SetPixelFormat(wgl.dc, index, &format) }.map_err(|err| err.to_string())?;
        wgl.context = unsafe { wglCreateContext(wgl.dc) }.map_err(|err| err.to_string())?;
        unsafe { wglMakeCurrent(wgl.dc, wgl.context) }.map_err(|err| err.to_string())?;
        Ok(wgl)
    }
}

impl Drop for WglContext {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_invalid() {
                let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
                let _ = wglDeleteContext(self.context);
            }
            if !self.dc.is_invalid() {
                ReleaseDC(Some(self.window), self.dc);
            }
            let _ = DestroyWindow(self.window);
        }
    }
}
