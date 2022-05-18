#![deny(rust_2018_idioms)]
mod reactor;
mod syscall;
pub mod waker;

use std::{
    collections::hash_map::Entry,
    fs::File,
    future::Future,
    io,
    os::unix::prelude::{AsRawFd, FromRawFd, RawFd},
    pin::Pin,
    sync::{atomic::AtomicBool, Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use reactor::Reactor;
use syscall::syscall;
use waker::{waker_ref, ArcWake};

pub struct Timer {
    deadline: Duration,
    file: Option<File>,
    first_call: bool,
}

impl Timer {
    #[inline]
    pub fn new(deadline: Duration) -> Timer {
        Timer {
            deadline,
            file: None,
            first_call: true,
        }
    }

    #[inline]
    fn start_timer(deadline: Duration) -> Result<RawFd, std::io::Error> {
        let timer_fd = syscall!(timerfd_create(
            libc::CLOCK_MONOTONIC,
            libc::TFD_NONBLOCK | libc::TFD_CLOEXEC,
        ))?;

        let spec = libc::itimerspec {
            it_value: libc::timespec {
                tv_sec: deadline.as_secs() as i64,
                tv_nsec: deadline.subsec_nanos() as i64,
            },
            it_interval: libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
        };

        syscall!(timerfd_settime(timer_fd, 0, &spec, std::ptr::null_mut()))?;

        Ok(timer_fd)
    }
}

impl Future for Timer {
    type Output = Result<(), std::io::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let reactor = reactor::current();

        if let Some(fd) = self.file.as_ref().map(|file| file.as_raw_fd()) {
            let mut lock = reactor.lock().unwrap();

            // If we are _polled_ again and the reactor still didn't consume our waker,
            // we should just replace the waker and don't register the timer again
            if let Entry::Occupied(mut e) = lock.wakers.entry(fd) {
                e.insert(cx.waker().clone());

                return Poll::Pending;
            }
        }

        if !self.first_call {
            return Poll::Ready(Ok(()));
        }

        let fd = Timer::start_timer(self.deadline)?;
        self.file = Some(unsafe { File::from_raw_fd(fd) });

        let event_flags = (libc::EPOLLIN | libc::EPOLLET | libc::EPOLLONESHOT) as u32;
        reactor
            .lock()
            .unwrap()
            .register_insterest(fd, event_flags, cx.waker().clone())?;

        self.first_call = false;

        Poll::Pending
    }
}

struct Waiter {
    is_woken: AtomicBool,
}

impl Waiter {
    #[inline]
    fn new() -> Arc<Waiter> {
        Arc::new(Waiter {
            is_woken: AtomicBool::new(true),
        })
    }

    #[inline]
    fn reset_waiter(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.is_woken.swap(false, Ordering::AcqRel)
    }
}

impl ArcWake for Waiter {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        use std::sync::atomic::Ordering;
        arc_self.is_woken.store(true, Ordering::Release);
    }
}

pub struct Executor {
    reactor: Arc<Mutex<Reactor>>,
}

impl Executor {
    #[inline]
    pub fn new() -> io::Result<Executor> {
        Ok(Executor {
            reactor: Arc::new(Mutex::new(Reactor::new()?)),
        })
    }

    pub fn block_on<F: Future>(&self, mut future: F) -> F::Output {
        let _enter = reactor::try_enter(Arc::clone(&self.reactor)).unwrap();
        let mut future = unsafe { Pin::new_unchecked(&mut future) };
        let waiter = Waiter::new();
        let waker = waker_ref(&waiter);
        let mut context = Context::from_waker(&*waker);

        loop {
            if waiter.reset_waiter() {
                if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                    return value;
                }
            }

            self.reactor.lock().unwrap().poll_events().unwrap();
        }
    }
}

#[inline]
pub async fn sleep(deadline: Duration) {
    let timer = Timer::new(deadline);
    timer.await.expect("should not fail");
}
