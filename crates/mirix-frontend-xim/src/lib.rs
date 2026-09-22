//! XIM server for X11 clients (niri + xwayland-satellite).
//!
//! Clients need `XMODIFIERS=@im=mirix`.

mod handler;
mod keymap;

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

use anyhow::{Context, Result};
use mirix_core::{ContextId, Instance};
use x11rb::connection::Connection;
use x11rb::rust_connection::RustConnection;
use xim::XimConnections;
use xim::x11rb::{HasConnection, X11rbServer};

use handler::Handler;
use keymap::Xkb;

pub const IM_NAME: &str = "mirix";

pub struct XimCtx {
    server: X11rbServer<RustConnection>,
    connections: XimConnections<ContextId>,
    xkb: Xkb,
}

impl XimCtx {
    pub fn connect() -> Result<Self> {
        let (conn, screen_num) =
            RustConnection::connect(None).context("DISPLAY / xwayland-satellite")?;
        let server = X11rbServer::init(conn, screen_num, IM_NAME, xim::ALL_LOCALES)
            .map_err(|err| anyhow::anyhow!("init XIM server: {err}"))?;
        let xkb = Xkb::from_system().context("XIM keymap")?;
        tracing::info!(
            name = IM_NAME,
            "XIM server ready (XMODIFIERS=@im={IM_NAME})"
        );
        Ok(Self {
            server,
            connections: XimConnections::new(),
            xkb,
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.server.conn().stream().as_fd().as_raw_fd()
    }

    /// # Safety
    /// `XimCtx` must outlive the event-loop source that holds this fd.
    pub unsafe fn borrowed_fd(&self) -> BorrowedFd<'static> {
        unsafe { BorrowedFd::borrow_raw(self.raw_fd()) }
    }

    pub fn dispatch(&mut self, instance: &mut Instance) -> Result<()> {
        loop {
            let e = match self.server.conn().poll_for_event() {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(err) => return Err(err.into()),
            };
            let mut handler = Handler {
                instance,
                xkb: &mut self.xkb,
            };
            if let Err(err) = self
                .server
                .filter_event(&e, &mut self.connections, &mut handler)
            {
                tracing::error!(%err, "xim filter_event");
            }
        }
        self.server.conn().flush()?;
        Ok(())
    }
}
