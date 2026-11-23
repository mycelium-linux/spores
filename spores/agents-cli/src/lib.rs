//! A WASI command wrapper for the aggregator's CLI probe.

wit_bindgen::generate!({
    path: "wit",
    world: "agents-cli",
    generate_all,
});

struct Component;

export!(Component);

impl exports::wasi::cli::run::Guest for Component {
    fn run() -> Result<(), ()> {
        let args = wasi::cli::environment::get_arguments();
        let args = if args.is_empty() {
            vec!["agents".to_string()]
        } else {
            args.into_iter().skip(1).collect()
        };
        match fungos::agents::cli::run(&args) {
            Ok(text) => {
                println!("{text}");
                Ok(())
            }
            Err(error) => {
                eprintln!("{error}");
                Err(())
            }
        }
    }
}
