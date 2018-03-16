extern crate jd4_5;
extern crate serde_yaml;

use jd4_5::compile::{self, Compiler, BinaryCompiler};
use jd4_5::sandbox::Sandbox;
use jd4_5::util::Pool;

pub fn main() {
    let gcc: BinaryCompiler = serde_yaml::from_str(
        r#"compiler_file: "/usr/bin/gcc"
compiler_args: ["gcc", "-static", "-O2", "-std=c99", "-o", "/out/foo", "/in/foo.c"]
code_file: "foo.c"
execute_file: "foo"
execute_args: ["foo"]"#).unwrap();
    let user_source = gcc.package(Box::new(*br#"#include <stdio.h>

int main(void) {
    printf("42\n");
}"#));
    let judge_source = gcc.package(Box::new(*br#"#define _POSIX_C_SOURCE 1
#include <stdio.h>

int main(void) {
    FILE *fp = fdopen(3, "r");
    if (!fp) {
        printf("open error\n");
        return 1;
    }
    int a;
    if (fscanf(fp, "%d", &a) != 1) {
        printf("read error\n");
        return 1;
    }
    printf("a = %d\n", a);
    return 0;
}"#));
    let pool = Pool::new();
    pool.put(Sandbox::new());
    pool.put(Sandbox::new());
    // These two can be parallelized...
    let user_target = gcc.compile(user_source, &pool);
    let judge_target = gcc.compile(judge_source, &pool);
    compile::run(user_target, judge_target, &pool);
}
