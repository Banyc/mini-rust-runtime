use std::{
    cell::RefCell,
    collections::VecDeque,
    marker::PhantomData,
    mem,
    pin::Pin,
    rc::Rc,
    task::{Context, RawWaker, RawWakerVTable, Waker},
};

use futures::{future::LocalBoxFuture, Future, FutureExt};

use crate::reactor::Reactor;

scoped_tls::scoped_thread_local!(pub(crate) static EX: Executor);

pub struct Executor {
    local_queue: TaskQueue,
    pub(crate) reactor: Rc<RefCell<Reactor>>,

    /// Make sure the type is `!Send` and `!Sync`.
    _marker: PhantomData<Rc<()>>,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    pub fn new() -> Self {
        Self {
            local_queue: TaskQueue::default(),
            reactor: Rc::new(RefCell::new(Reactor::default())),

            _marker: PhantomData,
        }
    }

    pub fn spawn(fut: impl Future<Output = ()> + 'static) {
        let t = Rc::new(Task {
            future: RefCell::new(fut.boxed_local()),
        });
        EX.with(|ex| {
            println!("[executor] spawn task; push task to executor queue");
            ex.local_queue.push(t)
        });
    }

    pub fn block_on<F, T, O>(&self, f: F) -> O
    where
        F: Fn() -> T,
        T: Future<Output = O> + 'static,
    {
        let _waker = waker_fn::waker_fn(|| {
            println!("[outer task] wake; no-op");
        });
        let cx = &mut Context::from_waker(&_waker);

        EX.set(self, || {
            let fut = f();
            pin_utils::pin_mut!(fut);
            loop {
                // return if the outer future is ready
                println!("[executor] poll outer future");
                if let std::task::Poll::Ready(t) = fut.as_mut().poll(cx) {
                    println!("[executor] outer future ready");
                    break t;
                }

                // consume all tasks
                while let Some(t) = self.local_queue.pop() {
                    println!("[executor] poll local task");
                    let future = t.future.borrow_mut();
                    let w = Rc::clone(&t).into_waker();
                    let mut context = Context::from_waker(&w);
                    let _ = Pin::new(future).as_mut().poll(&mut context);
                }

                // the outer future may ready now
                println!("[executor] poll outer future again");
                if let std::task::Poll::Ready(t) = fut.as_mut().poll(cx) {
                    println!("[executor] outer future ready");
                    break t;
                }

                // block for io
                println!("[executor] block for I/O");
                self.reactor.borrow_mut().wait();
            }
        })
    }
}

pub struct TaskQueue {
    queue: RefCell<VecDeque<Rc<Task>>>,
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskQueue {
    pub fn new() -> Self {
        const DEFAULT_TASK_QUEUE_SIZE: usize = 4096;
        Self::new_with_capacity(DEFAULT_TASK_QUEUE_SIZE)
    }
    pub fn new_with_capacity(capacity: usize) -> Self {
        Self {
            queue: RefCell::new(VecDeque::with_capacity(capacity)),
        }
    }

    pub(crate) fn push(&self, runnable: Rc<Task>) {
        self.queue.borrow_mut().push_back(runnable);
        println!("[executor queue] pushed task");
    }

    pub(crate) fn pop(&self) -> Option<Rc<Task>> {
        let task = self.queue.borrow_mut().pop_front();
        println!("[executor queue] popped task; is some {}", task.is_some());
        task
    }
}

pub struct Task {
    future: RefCell<LocalBoxFuture<'static, ()>>,
}

impl Task {
    fn into_waker(self: Rc<Self>) -> Waker {
        let ptr = Rc::into_raw(self) as *const ();
        let vtable = &Helper::VTABLE;
        unsafe { Waker::from_raw(RawWaker::new(ptr, vtable)) }
    }

    // TODO: Why it doesn't wake sometimes?
    fn wake(self: &Rc<Self>) {
        EX.with(|ex| {
            println!("[task] wake; push to executor queue");
            ex.local_queue.push(Rc::clone(self))
        });
    }
}

struct Helper;

impl Helper {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        Self::clone_waker,
        Self::wake,
        Self::wake_by_ref,
        Self::drop_waker,
    );

    unsafe fn clone_waker(data: *const ()) -> RawWaker {
        increase_refcount(data);
        let vtable = &Self::VTABLE;
        RawWaker::new(data, vtable)
    }

    unsafe fn wake(ptr: *const ()) {
        let rc = Rc::from_raw(ptr as *const Task);
        rc.wake();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        let rc = mem::ManuallyDrop::new(Rc::from_raw(ptr as *const Task));
        rc.wake();
    }

    unsafe fn drop_waker(ptr: *const ()) {
        drop(Rc::from_raw(ptr as *const Task));
    }
}

#[allow(clippy::redundant_clone)] // The clone here isn't actually redundant.
unsafe fn increase_refcount(data: *const ()) {
    // Retain Rc, but don't touch refcount by wrapping in ManuallyDrop
    let rc = mem::ManuallyDrop::new(Rc::<Task>::from_raw(data as *const Task));
    // Now increase refcount, but don't drop new refcount either
    let _rc_clone: mem::ManuallyDrop<_> = rc.clone();
}
