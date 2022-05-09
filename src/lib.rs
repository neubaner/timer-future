use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    mem::{ManuallyDrop, MaybeUninit},
    os::unix::prelude::{FromRawFd, RawFd},
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    time::{Duration, Instant},
};

pub trait ArcWake: Send + Sync {
    fn wake_by_ref(arc_self: &Arc<Self>);
    fn wake(self: Arc<Self>) {
        Self::wake_by_ref(&self)
    }
}

fn waker_vtable<W: ArcWake>() -> &'static RawWakerVTable {
    &RawWakerVTable::new(
        clone_arc_raw::<W>,
        wake_arc_raw::<W>,
        wake_arc_by_ref_raw::<W>,
        drop_arc_raw::<W>,
    )
}

unsafe fn clone_arc_raw<W: ArcWake>(data: *const ()) -> RawWaker {
    let arc = ManuallyDrop::new(Arc::from_raw(data.cast::<W>()));
    // Increase the reference counting
    let _ = arc.clone();
    RawWaker::new(data, waker_vtable::<W>())
}

unsafe fn wake_arc_raw<W: ArcWake>(data: *const ()) {
    let arc = Arc::from_raw(data.cast::<W>());
    ArcWake::wake(arc)
}

unsafe fn wake_arc_by_ref_raw<W: ArcWake>(data: *const ()) {
    let arc = ManuallyDrop::new(Arc::from_raw(data.cast::<W>()));
    ArcWake::wake_by_ref(&arc);
}

unsafe fn drop_arc_raw<W: ArcWake>(data: *const ()) {
    let arc = Arc::from_raw(data.cast::<W>());
    drop(arc);
}

pub fn waker<W: ArcWake>(arc: Arc<W>) -> Waker {
    let ptr = Arc::into_raw(arc).cast::<()>();
    unsafe { Waker::from_raw(RawWaker::new(ptr, waker_vtable::<W>())) }
}

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
    first_call: bool,
    reactor: Arc<Mutex<Reactor>>,
}

impl Timer {
    pub fn new(deadline: Duration, reactor: Arc<Mutex<Reactor>>) -> Timer {
        Timer {
            deadline,
            fd: None,
            first_call: true,
            reactor,
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

        syscall!(timerfd_settime(timer_fd, 0, &spec, std::ptr::null_mut()))?;

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

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.first_call {
            Poll::Ready(Ok(()))
        } else {
            let fd = Timer::start_timer(self.deadline)?;
            self.fd = Some(fd);

            let event_flags = (libc::EPOLLIN | libc::EPOLLET | libc::EPOLLONESHOT) as u32;
            self.reactor
                .lock()
                .unwrap()
                .register_insterest(fd, event_flags, cx.waker().clone())?;

            self.first_call = false;

            Poll::Pending
        }
    }
}

pub struct Task {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + 'static + Send>>>>,
    // Wrapping Sender in a mutex to make it Sync.
    // Maybe there's a better way to handle this?
    sender: Mutex<mpsc::Sender<Arc<Task>>>,
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self
            .sender
            .lock()
            .unwrap()
            .send(Arc::clone(arc_self))
            .expect("Could not send message");
    }
}

pub struct Executor {
    sender: mpsc::Sender<Arc<Task>>,
    receiver: mpsc::Receiver<Arc<Task>>,
    reactor: Arc<Mutex<Reactor>>,
}

impl Executor {
    pub fn new(reactor: Arc<Mutex<Reactor>>) -> Executor {
        let (sender, receiver) = mpsc::channel();
        Executor {
            sender,
            receiver,
            reactor,
        }
    }

    pub fn block_on<F: Future<Output = ()> + 'static + Send>(self, future: F) -> F::Output {
        let future = Box::pin(future);
        let task = Arc::new(Task {
            future: Mutex::new(Some(future)),
            sender: Mutex::new(self.sender.clone()),
        });

        self.sender.send(task).unwrap();

        loop {
            if self.wait_for_one_task().is_err() {
                break;
            }

            self.reactor.lock().unwrap().poll_events().unwrap();
        }
    }

    pub fn wait_for_one_task(&self) -> Result<(), mpsc::RecvError> {
        match self.receiver.recv() {
            Ok(task) => {
                let mut future_slot = task.future.lock().unwrap();

                if let Some(mut future) = future_slot.take() {
                    let waker = waker(Arc::clone(&task));
                    let mut context = Context::from_waker(&waker);

                    if future.as_mut().poll(&mut context).is_pending() {
                        *future_slot = Some(future);
                    }
                }

                Ok(())
            }
            Err(err) => Err(err),
        }
    }
}

pub struct Reactor {
    wakers: HashMap<RawFd, Waker>,
    epoll_fd: RawFd,
}

impl Reactor {
    pub fn new() -> Result<Arc<Mutex<Reactor>>, std::io::Error> {
        let epoll_fd = syscall!(epoll_create1(libc::EPOLL_CLOEXEC))?;

        Ok(Arc::new(Mutex::new(Reactor {
            epoll_fd,
            wakers: HashMap::new(),
        })))
    }

    fn register_insterest(
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

    fn poll_events(&mut self) -> Result<(), std::io::Error> {
        const MAX_EVENTS: usize = 32;

        let mut events: [MaybeUninit<libc::epoll_event>; MAX_EVENTS] =
            unsafe { MaybeUninit::uninit().assume_init() };
        let events_ptr = events.as_mut_ptr().cast::<libc::epoll_event>();

        let nfds = syscall!(epoll_wait(self.epoll_fd, events_ptr, MAX_EVENTS as i32, -1))?;

        for i in 0..nfds {
            let event: libc::epoll_event = unsafe { events[i as usize].assume_init() };
            let fd = event.u64 as RawFd;

            if let Some(waker) = self.wakers.remove(&fd) {
                waker.wake();
            }
        }

        Ok(())
    }
}

pub async fn sleep(deadline: Duration, reactor: Arc<Mutex<Reactor>>) -> Duration {
    let now = Instant::now();
    let timer = Timer::new(deadline, reactor);
    timer.await.expect("should not fail");
    now.elapsed()
}
