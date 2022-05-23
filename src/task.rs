use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll}, mem,
};

use crate::{Spawner, waker::{ArcWake, waker_ref}};

pub struct RawTaskVTable {
    pub(crate) poll: fn(raw_task: Arc<RawTask>),
    pub(crate) try_read_future : fn(raw_task: Arc<RawTask>, dst: *mut ())
}

fn raw_task_vtable<F: Future>() -> &'static RawTaskVTable {
    &RawTaskVTable { poll: poll::<F>, try_read_future: try_read_output::<F> }
}

fn poll<F: Future>(raw_task: Arc<RawTask>) {
    let cell = unsafe { &mut *raw_task.cell_ptr.cast::<Cell<F>>() };

    if let Cell::Future(future) = cell {
        let future = unsafe { Pin::new_unchecked(future) };
        let waker = waker_ref(&raw_task);
        let mut cx = Context::from_waker(&*waker);
        let poll_result = future.poll(&mut cx);
        println!("polled task: {}", poll_result.is_ready());
    
        if let Poll::Ready(value) = poll_result {
            *cell = Cell::Result(value);
        }
    }
}

fn try_read_output<F: Future>(raw_task: Arc<RawTask>, dst: *mut()) {
    let poll_result = unsafe { &mut *dst.cast::<Poll<F::Output>>() };
    let cell = unsafe { &mut *raw_task.cell_ptr.cast::<Cell<F>>() };
    
    if cell.is_result() {
        let cell = mem::replace(cell, Cell::Consumed);
        if let Cell::Result(result) = cell {
            *poll_result = Poll::Ready(result);
        }
    }
}

pub enum Cell<F: Future> {
    Future(F),
    Result(F::Output),
    Consumed
}

impl<F: Future> Cell<F> {
    fn is_result(&self) -> bool {
        matches!(self, &Cell::Result(_))
    }
}

pub struct RawTask {
    pub(crate) vtable: &'static RawTaskVTable,
    pub(crate) spawner: Arc<Spawner>,
    pub(crate) cell_ptr: *mut (),
}

impl RawTask {
    pub fn new<F: Future>(future: F, spawner: Arc<Spawner>) -> RawTask {
        let cell = Box::new(Cell::Future(future));
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
        // TODO: Need to implement drop on task's vtable
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

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut result = Poll::Pending;
        (self.raw_task.vtable.try_read_future)(Arc::clone(&self.raw_task), &mut result as *mut _ as *mut ());

        result
    }
}
