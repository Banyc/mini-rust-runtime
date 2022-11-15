use std::{
    cell::RefCell,
    collections::VecDeque,
    marker::PhantomData,
    mem,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use futures::{executor::block_on, future::LocalBoxFuture, Future, FutureExt};

use crate::reactor::Reactor;

// A global executor instance for a thread at which `Executor::block_on` is called.
scoped_tls::scoped_thread_local!(pub(crate) static EX: Executor);

pub struct Executor {
    woken_tasks: TaskQueue,
    pub(crate) reactor: Rc<RefCell<Reactor>>,

    /// Make sure the type is `!Send` and `!Sync`.
    _marker: PhantomData<Rc<()>>,
}

impl Default for Executor {
    fn default() -> Self {
        Self {
            woken_tasks: TaskQueue::default(),
            reactor: Rc::new(RefCell::new(Reactor::default())),

            _marker: PhantomData,
        }
    }
}

impl Executor {
    pub fn spawn(fut: impl Future<Output = ()> + 'static) {
        let task = Rc::new(Task {
            future: RefCell::new(fut.boxed_local()),
        });
        EX.with(|ex| {
            println!("[executor] spawn task; push task to executor queue");
            ex.woken_tasks.push(task)
        });
    }

    pub fn block_on<F, T, O>(f: F) -> O
    where
        F: Fn() -> T,
        T: Future<Output = O> + 'static,
    {
        if EX.is_set() {
            panic!("cannot call `block_on` inside of an executor");
        }
        let ex = Executor::default();
        ex.block_on_(f)
    }

    #[inline]
    pub(crate) fn reactor() -> Rc<RefCell<Reactor>> {
        EX.with(|ex| Rc::clone(&ex.reactor))
    }

    pub fn block_on_<F, T, O>(&self, f: F) -> O
    where
        F: Fn() -> T,
        T: Future<Output = O> + 'static,
    {
        let waker_noop = waker_fn::waker_fn(|| {
            println!("[outer task] wake; no-op");
        });
        let f_cx = &mut Context::from_waker(&waker_noop);

        EX.set(self, || {
            let f_fut = f();
            pin_utils::pin_mut!(f_fut);
            loop {
                // return if the outer future is ready
                println!("[executor] poll outer future");
                if let Poll::Ready(out) = f_fut.as_mut().poll(f_cx) {
                    println!("[executor] outer future ready");
                    break out;
                }

                // consume all woken tasks
                while let Some(task) = self.woken_tasks.pop() {
                    println!("[executor] polled woken task");
                    let fut = task.future.borrow_mut();

                    // If the polling returns `Poll::Ready`, the task is done.
                    // If the polling returns `Poll::Pending`, the task has not been done.
                    // - We need to provide a waker for the task to the reactor.
                    let waker = Rc::clone(&task).into_waker();
                    let mut cx = Context::from_waker(&waker);
                    let _ = Pin::new(fut).as_mut().poll(&mut cx);
                }

                // the outer future may ready now
                println!("[executor] poll outer future again");
                if let Poll::Ready(out) = f_fut.as_mut().poll(f_cx) {
                    println!("[executor] outer future ready");
                    break out;
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
        const DEFAULT_TASK_QUEUE_SIZE: usize = 4096;
        Self::new_with_capacity(DEFAULT_TASK_QUEUE_SIZE)
    }
}

impl TaskQueue {
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
        let vtable = &RawWakerVTableBuilder::VTABLE;
        unsafe { Waker::from_raw(RawWaker::new(ptr, vtable)) }
    }

    fn wake(self: &Rc<Self>) {
        EX.with(|ex| {
            println!("[task] wake; push to woken tasks");
            ex.woken_tasks.push(Rc::clone(self))
        });
    }
}

struct RawWakerVTableBuilder;

impl RawWakerVTableBuilder {
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
