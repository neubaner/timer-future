use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    mem::MaybeUninit,
    os::unix::prelude::RawFd,
    sync::{Arc, Mutex},
    task::Waker,
};

use crate::syscall::syscall;

type Handle = Arc<Mutex<Reactor>>;

thread_local! {
    static REACTOR: RefCell<Option<Handle>> = RefCell::new(None)
}

pub(crate) struct EnterGuard(Option<Handle>);

impl Drop for EnterGuard {
    fn drop(&mut self) {
        REACTOR.with(|reactor| {
            *reactor.borrow_mut() = self.0.take();
        });
    }
}

pub(crate) fn try_enter(handle: Handle) -> Option<EnterGuard> {
    REACTOR
        .try_with(|reactor| {
            let old = reactor.borrow_mut().replace(handle);
            EnterGuard(old)
        })
        .ok()
}

pub(crate) fn current() -> Handle {
    REACTOR
        .try_with(|reactor| reactor.borrow().clone().unwrap())
        .unwrap()
}

pub(crate) struct Reactor {
    pub(crate) wakers: HashMap<RawFd, Waker>,
    pub(crate) epoll_fd: RawFd,
}

impl Reactor {
    #[inline]
    pub fn new() -> io::Result<Reactor> {
        let epoll_fd = syscall!(epoll_create1(libc::EPOLL_CLOEXEC))?;

        Ok(Reactor {
            epoll_fd,
            wakers: HashMap::new(),
        })
    }

    pub fn register_insterest(
        &mut self,
        fd: RawFd,
        event_flags: u32,
        waker: Waker,
    ) -> Result<(), std::io::Error> {
        let mut ev = libc::epoll_event {
            events: event_flags,
            u64: fd as u64,
        };

        let _ = syscall!(epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ev))?;

        self.wakers.insert(fd, waker);

        Ok(())
    }

    pub fn poll_events(&mut self) -> Result<(), std::io::Error> {
        const MAX_EVENTS: usize = 32;

        let mut events: [MaybeUninit<libc::epoll_event>; MAX_EVENTS] =
            unsafe { MaybeUninit::uninit().assume_init() };
        let events_ptr = events.as_mut_ptr().cast::<libc::epoll_event>();

        loop {
            let nfds = match syscall!(epoll_wait(self.epoll_fd, events_ptr, MAX_EVENTS as i32, -1))
            {
                Ok(nfds) => nfds,
                Err(ref error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };

            for i in 0..nfds {
                let event: libc::epoll_event = unsafe { events[i as usize].assume_init() };
                let fd = event.u64 as RawFd;

                if let Some(waker) = self.wakers.remove(&fd) {
                    waker.wake();
                }
            }

            return Ok(());
        }
    }
}
