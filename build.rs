fn main() {
	#[cfg(not(target_os = "linux"))]
	compile_error!("Sorry, this crate only supports linux systems.");
}
