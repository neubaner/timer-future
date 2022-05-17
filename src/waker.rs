use std::{
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    ptr,
    sync::Arc,
    task::{RawWaker, RawWakerVTable, Waker},
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

#[inline]
pub fn waker<W: ArcWake>(arc: Arc<W>) -> Waker {
    let ptr = Arc::into_raw(arc).cast::<()>();
    unsafe { Waker::from_raw(RawWaker::new(ptr, waker_vtable::<W>())) }
}

pub struct WakerRef<'a> {
    waker: ManuallyDrop<Waker>,
    _has: PhantomData<&'a ()>,
}

impl<'a> WakerRef<'a> {
    pub fn new(waker: &'a Waker) -> Self {
        let waker = ManuallyDrop::new(unsafe { ptr::read(waker) });

        Self {
            waker,
            _has: PhantomData,
        }
    }

    pub fn new_uowned(waker: ManuallyDrop<Waker>) -> Self {
        Self {
            waker,
            _has: PhantomData,
        }
    }
}

impl Deref for WakerRef<'_> {
    type Target = Waker;

    fn deref(&self) -> &Self::Target {
        &self.waker
    }
}

#[inline]
pub fn waker_ref<W: ArcWake>(arc: &Arc<W>) -> WakerRef<'_> {
    let ptr = Arc::as_ptr(arc).cast::<()>();
    let waker =
        ManuallyDrop::new(unsafe { Waker::from_raw(RawWaker::new(ptr, waker_vtable::<W>())) });
    WakerRef::new_uowned(waker)
}
