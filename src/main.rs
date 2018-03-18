extern crate jd4_5;

use jd4_5::compile;
use jd4_5::config::Registry;
use jd4_5::sandbox::Sandbox;
use jd4_5::util::Pool;

pub fn main() {
    let pool = Pool::new();
    pool.put(Sandbox::new());
    pool.put(Sandbox::new());
    let gcc = Registry::builtin().get_compiler("c").unwrap();

    let user_source = br#"#include <stdio.h>

int main(void) {
    printf("42\n");
}"#;
    let judge_source = br#"#define _POSIX_C_SOURCE 1
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
}"#;
    // These two can be parallelized...
    let user_target = gcc.compile(user_source, &pool);
    let judge_target = gcc.compile(judge_source, &pool);
    compile::run(user_target, judge_target, &pool);
}
