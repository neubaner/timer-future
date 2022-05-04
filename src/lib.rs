use std::{sync::Arc, task::{Waker, RawWakerVTable, RawWaker}, mem::{self, ManuallyDrop}};

pub trait ArcWake : Send + Sync {
    fn wake_by_ref(self: &Arc<Self>);
    fn wake(self: Arc<Self>) {
        Self::wake_by_ref(&self)
    }
}

fn waker_vtable<W: ArcWake>() -> &'static RawWakerVTable {
    &RawWakerVTable::new(
        clone_arc_raw::<W>,
        wake_arc_raw::<W>,
        wake_arc_by_ref_raw::<W>,
        drop_arc_raw::<W>
    )
}

unsafe fn clone_arc_raw<W: ArcWake>(data: *const ()) -> RawWaker {
    let arc = Arc::from_raw(data.cast::<W>());
    // Increase the reference counting
    mem::forget(arc.clone());
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
    drop(Arc::from_raw(data.cast::<W>()));
}

pub fn waker<W: ArcWake>(arc: Arc<W>) -> Waker {
    let ptr = Arc::into_raw(arc).cast::<()>();
    unsafe { Waker::from_raw(RawWaker::new(ptr, waker_vtable::<W>())) }
}