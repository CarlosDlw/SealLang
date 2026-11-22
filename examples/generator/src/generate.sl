// Generators run in a staging tree with only their declared inputs and their
// own executable visible. They write exactly the declared outputs.
extern "c" {
    func getenv(name: *raw u8) -> *raw u8;
    func fopen(path: *raw u8, mode: *raw u8) -> *raw u8;
    func fgets(buffer: *raw mut u8, size: c_int, stream: *raw u8) -> *raw u8;
    func fputs(text: *raw u8, stream: *raw u8) -> c_int;
    func fclose(stream: *raw u8) -> c_int;
}

func main() -> i32 {
    unsafe {
        let input = getenv(c"SEAL_GENERATOR_INPUTS");
        if input.is_null() {
            return 1;
        }
        let source = fopen(input, c"r");
        if source.is_null() {
            return 2;
        }
        let output = fopen(c"generated/value.sl", c"w");
        if output.is_null() {
            fclose(source);
            return 3;
        }
        let mut buffer: [u8; 256] = [0u8; 256];
        while !fgets(&mut buffer[0] as *raw mut u8, 256, source).is_null() {
            if fputs(&buffer[0] as *raw u8, output) < 0 {
                fclose(source);
                fclose(output);
                return 4;
            }
        }
        fclose(source);
        fclose(output);
    }
    0
}
