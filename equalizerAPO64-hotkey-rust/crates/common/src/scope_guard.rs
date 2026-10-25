//! Port of `helpers/ScopeGuard.h` (RAII exit helpers).

pub struct ScopeGuard<F: FnMut()> {
    func: Option<F>,
}

impl<F: FnMut()> ScopeGuard<F> {
    pub fn new(func: F) -> Self {
        Self { func: Some(func) }
    }

    pub fn dismiss(&mut self) {
        self.func = None;
    }
}

impl<F: FnMut()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(func) = &mut self.func {
            func();
        }
    }
}

#[macro_export]
macro_rules! scope_exit {
    ($body:block) => {
        let mut __guard = $crate::scope_guard::ScopeGuard::new(|| $body);
        let _ = &mut __guard;
    };
}
