use std::{
    fs::File,
    future::Future,
    mem::{self, MaybeUninit},
    os::unix::prelude::{FromRawFd, RawFd},
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

// From mio https://github.com/tokio-rs/mio/blob/1667a7027382bd43470bc43e5982531a2e14b7ba/src/sys/unix/mod.rs
macro_rules! syscall {
  ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
      let res = unsafe { libc::$fn($($arg, )*) };
      if res == -1 {
          Err(std::io::Error::last_os_error())
      } else {
          Ok(res)
      }
  }};
}

pub struct Timer {
    deadline: Duration,
    fd: Option<RawFd>,
    is_completed: bool,
}

impl Timer {
    pub fn new(deadline: Duration) -> Timer {
        Timer {
            deadline,
            fd: None,
            is_completed: false,
        }
    }

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

        let _ = syscall!(timerfd_settime(timer_fd, 0, &spec, std::ptr::null_mut()))?;

        Ok(timer_fd)
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            let _ = unsafe { File::from_raw_fd(fd) };
        }
    }
}

impl Future for Timer {
    type Output = Result<(), std::io::Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_completed {
            Poll::Ready(Ok(()))
        } else {
            let fd_result = Timer::start_timer(self.deadline);
            let fd = match fd_result {
                Ok(fd) => fd,
                Err(err) => return Poll::Ready(Err(err))
            };
            self.fd = Some(fd);
            
            // TODO: register the timer into the epoll with a Waker
            Poll::Pending
        }
    }
}

fn main() -> Result<(), std::io::Error> {
    let deadline = Duration::from_secs(3);

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

    let _ = syscall!(timerfd_settime(timer_fd, 0, &spec, std::ptr::null_mut()))?;

    let epoll_fd = syscall!(epoll_create1(libc::EPOLL_CLOEXEC))?;

    let mut ev = libc::epoll_event {
        events: (libc::EPOLLIN | libc::EPOLLET | libc::EPOLLONESHOT) as u32,
        u64: timer_fd as u64,
    };

    let _ = syscall!(epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, timer_fd, &mut ev))?;

    const MAX_EVENTS: usize = 32;
    let mut events: [MaybeUninit<libc::epoll_event>; MAX_EVENTS] =
        unsafe { MaybeUninit::uninit().assume_init() };
    let events_ptr = events.as_mut_ptr().cast::<libc::epoll_event>();

    let now = Instant::now();

    loop {
        println!("Entering epoll_wait");
        let nfds = syscall!(epoll_wait(epoll_fd, events_ptr, MAX_EVENTS as i32, -1))?;
        println!("Got {} events!", nfds);

        for i in 0..nfds {
            let event: libc::epoll_event = unsafe { mem::transmute(events[i as usize]) };

            if event.u64 == timer_fd as u64 {
                println!(
                    "Timer ({}) expired!. Rust Instant API shows {} seconds",
                    timer_fd,
                    now.elapsed().as_secs_f64()
                );
                return Ok(());
            }
        }
    }
}
