extern crate fnv;

mod ffi;

use std::os::raw::*;
use std::ffi::{CString, CStr};
use std::mem::transmute;
use std::marker::PhantomData;

use fnv::FnvHashMap as HashMap; // faster than std HashMap for small keys

use ffi::*;

pub fn run<'a, T: 'a,
	I: FnMut(MyUnique<WebView<'a, T>>),
	F: FnMut(&mut WebView<'a, T>, &str, &mut T) + 'a,
>(
	title: &str, url: &str, size: Option<(i32, i32)>, resizable: bool, debug: bool, mut init_cb: I, ext_cb: F, user_data: T
) -> (T, bool) {
	let (width, height) = size.unwrap_or((800, 600));
	let fullscreen = size.is_none();
	let title = CString::new(title).unwrap();
	let url = CString::new(url).unwrap();
	let mut handler_data = Box::new(HandlerData {
		ext_cb: Box::new(ext_cb),
		index: 0,
		dispatched_cbs: Default::default(),
		user_data
	});
	let webview = unsafe {
		wrapper_webview_new(
			title.as_ptr(), url.as_ptr(), width, height, resizable as c_int, debug as c_int,
			Some(transmute(handler_ext::<T> as ExternalInvokeFn<T>)),
			&mut *handler_data as *mut _ as *mut c_void
		)
	};
	if webview.is_null() {
		(handler_data.user_data, false)
	} else {
		unsafe { webview_set_fullscreen(webview, fullscreen as _); }
		init_cb(MyUnique(webview as _));
		unsafe {
			while webview_loop(webview, 1) == 0 {}
			webview_exit(webview);
			wrapper_webview_free(webview);
		}
		(handler_data.user_data, true)
	}
}

struct HandlerData<'a, T: 'a> {
	ext_cb: Box<FnMut(&mut WebView<'a, T>, &str, &mut T) + 'a>,
	index: usize,
	dispatched_cbs: HashMap<usize, Box<FnMut(&mut WebView<'a, T>, &mut T) + 'a>>,
	user_data: T
}

pub struct WebView<'a, T: 'a>(PhantomData<&'a mut T>);

pub struct MyUnique<T>(*mut T);
unsafe impl<T> Send for MyUnique<T> {}
unsafe impl<T> Sync for MyUnique<T> {}

impl<'a, T> MyUnique<WebView<'a, T>> {
	#[inline(always)]
	pub fn dispatch<F: FnMut(&mut WebView<'a, T>, &mut T) + 'a>(&self, f: F) {
		unsafe { &mut *self.0 }.dispatch(f);
	}
}

impl<'a, T> WebView<'a, T> {
	#[inline(always)]
	fn erase(&mut self) -> *mut CWebView { self as *mut _ as *mut _ }

	#[inline(always)]
	fn get_userdata(&mut self) -> &mut HandlerData<T> {
		let user_data = unsafe { wrapper_webview_get_userdata(self.erase()) };
		let data: &mut HandlerData<T> = unsafe { &mut *(user_data as *mut HandlerData<T>) };
		data
	}

	pub fn terminate(&mut self) {
		unsafe { webview_terminate(self.erase()) }
	}

	pub fn dispatch<F: FnMut(&mut WebView<'a, T>, &mut T) + 'a>(&'a mut self, f: F) {
		let erased = self.erase();
		let index = {
			let data = self.get_userdata();
			let index = data.index;
			data.index += 1;
			data.dispatched_cbs.insert(index, Box::new(f));
			index
		};
		unsafe {
			webview_dispatch(erased, Some(transmute(handler_dispatch as DispatchFn<T>)), index as _)
		}
	}

	pub fn eval(&mut self, js: &str) -> i32 {
		let js = CString::new(js).unwrap();
		unsafe { webview_eval(self.erase(), js.as_ptr()) }
	}

	pub fn inject_css(&mut self, css: &str) -> i32 {
		let css = CString::new(css).unwrap();
		unsafe { webview_inject_css(self.erase(), css.as_ptr()) }
	}
}

type ExternalInvokeFn<'a, T> = extern "system" fn(webview: *mut WebView<'a, T>, arg: *const c_char);
type DispatchFn<'a, T> = extern "system" fn(webview: *mut WebView<'a, T>, arg: *mut c_void);


extern "system" fn handler_dispatch<'a, T>(webview: *mut WebView<'a, T>, arg: *mut c_void) {
	let data = unsafe { (*webview).get_userdata() };
	let i = arg as _;
	use std::collections::hash_map::Entry;
	if let Entry::Occupied(mut e) = data.dispatched_cbs.entry(i) {
		e.get_mut()(unsafe { &mut *webview }, &mut data.user_data);
		e.remove_entry();
	} else {
		unreachable!();
	}
}

extern "system" fn handler_ext<'a, T>(webview: *mut WebView<'a, T>, arg: *const c_char) {
	let data = unsafe { (*webview).get_userdata() };
	let arg = unsafe { CStr::from_ptr(arg) }.to_string_lossy().to_string();
	(data.ext_cb)(unsafe { &mut *webview }, &arg, &mut data.user_data);
}
