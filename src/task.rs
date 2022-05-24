use std::{
    future::Future,
    marker::PhantomData,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use crate::{
    waker::{waker_ref, ArcWake},
    Spawner,
};

pub struct RawTaskVTable {
    pub(crate) poll: fn(raw_task: Arc<RawTask>),
    try_read_future: fn(raw_task: Arc<RawTask>, dst: *mut (), waker: &Waker),
    drop_task: fn(raw_task: &mut RawTask),
}

fn raw_task_vtable<F: Future>() -> &'static RawTaskVTable {
    &RawTaskVTable {
        poll: poll::<F>,
        try_read_future: try_read_output::<F>,
        drop_task: drop_task::<F>,
    }
}

fn poll<F: Future>(raw_task: Arc<RawTask>) {
    let cell = unsafe { &mut *raw_task.cell_ptr.cast::<Cell<F>>() };

    if let FutureState::Running(future) = &mut cell.future_state {
        let future = unsafe { Pin::new_unchecked(future) };
        let waker = waker_ref(&raw_task);
        let mut cx = Context::from_waker(&*waker);
        let poll_result = future.poll(&mut cx);

        if let Poll::Ready(value) = poll_result {
            (*cell).future_state = FutureState::Completed(value);

            if let Some(waker) = cell.waker.take() {
                waker.wake();
            }
        }
    }
}

fn try_read_output<F: Future>(raw_task: Arc<RawTask>, dst: *mut (), waker: &Waker) {
    let poll_result = unsafe { &mut *dst.cast::<Poll<F::Output>>() };
    let cell = unsafe { &mut *raw_task.cell_ptr.cast::<Cell<F>>() };

    if let Some(cell_waker) = cell.waker.as_ref() {
        if !cell_waker.will_wake(waker) {
            cell.waker = Some(waker.clone());
        }
    } else {
        cell.waker = Some(waker.clone())
    }

    if cell.is_completed() {
        let future_or_result = mem::replace(&mut cell.future_state, FutureState::Consumed);
        if let FutureState::Completed(result) = future_or_result {
            *poll_result = Poll::Ready(result);
        }
    }
}

fn drop_task<F: Future>(raw_task: &mut RawTask) {
    let cell = unsafe { &mut *raw_task.cell_ptr.cast::<Cell<F>>() };
    cell.drop();
}

pub enum FutureState<F: Future> {
    Running(F),
    Completed(F::Output),
    Consumed,
}

pub struct Cell<F: Future> {
    waker: Option<Waker>,
    future_state: FutureState<F>,
}

impl<F: Future> Cell<F> {
    fn drop(&mut self) {
        self.future_state = FutureState::Consumed;
        self.waker = None;
    }
}

impl<F: Future> Cell<F> {
    fn is_completed(&self) -> bool {
        matches!(&self.future_state, &FutureState::Completed(_))
    }
}

pub struct RawTask {
    pub(crate) vtable: &'static RawTaskVTable,
    pub(crate) spawner: Arc<Spawner>,
    pub(crate) cell_ptr: *mut (),
}

impl RawTask {
    pub fn new<F: Future>(future: F, spawner: Arc<Spawner>) -> RawTask {
        let cell = Box::new(Cell {
            future_state: FutureState::Running(future),
            waker: None,
        });

        let cell_ptr = Box::into_raw(cell).cast::<()>();

        RawTask {
            vtable: raw_task_vtable::<F>(),
            cell_ptr,
            spawner,
        }
    }
}

impl Drop for RawTask {
    fn drop(&mut self) {
        (self.vtable.drop_task)(self);
    }
}

unsafe impl Send for RawTask {}
unsafe impl Sync for RawTask {}

impl ArcWake for RawTask {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let mut queue = arc_self.spawner.queue.lock().unwrap();
        queue.push_back(Arc::clone(arc_self));
    }
}

pub struct JoinHandle<T> {
    raw_task: Arc<RawTask>,
    _has: PhantomData<T>,
}

impl<T> JoinHandle<T> {
    pub fn new(raw_task: Arc<RawTask>) -> JoinHandle<T> {
        JoinHandle {
            raw_task,
            _has: PhantomData,
        }
    }
}

impl<T> Unpin for JoinHandle<T> {}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut result = Poll::Pending;
        (self.raw_task.vtable.try_read_future)(
            Arc::clone(&self.raw_task),
            &mut result as *mut _ as *mut (),
            cx.waker(),
        );

        result
    }
}
