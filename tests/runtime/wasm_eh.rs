/// PoC: Verify wasmtime 41 supports WASM Exception Handling proposal.
///
/// Tests:
/// 1. Engine config with wasm_exceptions(true) works alongside tail_call
/// 2. throw/catch round-trip preserves i64 payload
/// 3. Uncaught exception becomes a trap visible to host
/// 4. try_table with catch_all
use wasm_encoder::{
    BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
    Module, TagKind, TagSection, TagType, TypeSection, ValType,
};
use wasmtime::{Config, Engine, Store};

/// Build a WASM module that:
///   - defines tag 0: (i64) -> ()
///   - func "main"(): throw tag 0 with i64 payload, catch it via try_table, assert value
fn build_throw_catch_module() -> Vec<u8> {
    let mut module = Module::new();

    // --- Type section ---
    // type 0: () -> ()           (main signature)
    // type 1: (i64) -> ()        (tag signature)
    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![]);
    types.ty().function(vec![ValType::I64], vec![]);
    module.section(&types);

    // --- Function section ---
    // func 0 = main : type 0
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

    // --- Tag section ---
    // tag 0 : type 1 (i64 payload)
    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: 1,
    });
    module.section(&tags);

    // --- Export section ---
    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 0);
    module.section(&exports);

    // --- Code section ---
    // main():
    //   try_table (catch tag0 $caught) do
    //     i64.const 42
    //     throw tag0
    //   end
    //   unreachable          ;; should not reach here
    //   $caught:             ;; catch delivers i64 on stack
    //   ;; stack: [i64]
    //   i64.const 42
    //   i64.ne
    //   if
    //     unreachable        ;; payload mismatch = test failure
    //   end
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]); // no locals

    // The try_table catches tag0 and branches to label 1 (the outer block)
    // with the i64 payload on the stack.
    //
    // Structure:
    //   block (result i64)        ;; label 0 — landing pad
    //     try_table () (catch tag0 0)  ;; label 1
    //       i64.const 42
    //       throw tag0
    //     end
    //     unreachable             ;; throw always taken
    //   end
    //   ;; stack: [i64] from catch
    //   i64.const 42
    //   i64.ne
    //   if
    //     unreachable
    //   end
    f.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));
    f.instruction(&Instruction::TryTable(
        BlockType::Empty,
        vec![wasm_encoder::Catch::One { tag: 0, label: 0 }].into(),
    ));
    f.instruction(&Instruction::I64Const(42));
    f.instruction(&Instruction::Throw(0));
    f.instruction(&Instruction::End); // end try_table
    f.instruction(&Instruction::Unreachable);
    f.instruction(&Instruction::End); // end block — caught value on stack
                                      // stack: [i64]
    f.instruction(&Instruction::I64Const(42));
    f.instruction(&Instruction::I64Ne);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::Unreachable); // payload mismatch
    f.instruction(&Instruction::End); // end if
    f.instruction(&Instruction::End); // end function

    code.function(&f);
    module.section(&code);

    module.finish()
}

/// Build a module where throw is NOT caught — should trap.
fn build_uncaught_throw_module() -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![]);
    types.ty().function(vec![ValType::I64], vec![]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: 1,
    });
    module.section(&tags);

    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 0);
    module.section(&exports);

    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]);
    f.instruction(&Instruction::I64Const(99));
    f.instruction(&Instruction::Throw(0));
    f.instruction(&Instruction::End);
    code.function(&f);
    module.section(&code);

    module.finish()
}

/// Build a module testing catch_all (no payload).
fn build_catch_all_module() -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![]);
    types.ty().function(vec![ValType::I64], vec![]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: 1,
    });
    module.section(&tags);

    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 0);
    module.section(&exports);

    // main():
    //   block            ;; label 0 — catch_all landing
    //     try_table () (catch_all 0)
    //       i64.const 77
    //       throw tag0
    //     end
    //     unreachable
    //   end
    //   ;; catch_all delivers no payload — just survived
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]);
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::TryTable(
        BlockType::Empty,
        vec![wasm_encoder::Catch::All { label: 0 }].into(),
    ));
    f.instruction(&Instruction::I64Const(77));
    f.instruction(&Instruction::Throw(0));
    f.instruction(&Instruction::End); // end try_table
    f.instruction(&Instruction::Unreachable);
    f.instruction(&Instruction::End); // end block
    f.instruction(&Instruction::End); // end function
    code.function(&f);
    module.section(&code);

    module.finish()
}

/// Build a module with throw in callee, catch in caller — cross-function unwinding.
fn build_cross_function_throw_catch() -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![]); // type 0: () -> ()
    types.ty().function(vec![ValType::I64], vec![]); // type 1: (i64) -> ()
    module.section(&types);

    // func 0 = thrower, func 1 = main
    let mut functions = FunctionSection::new();
    functions.function(0); // thrower: type 0
    functions.function(0); // main: type 0
    module.section(&functions);

    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: 1,
    });
    module.section(&tags);

    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 1);
    module.section(&exports);

    let mut code = CodeSection::new();

    // thrower():
    //   i64.const 123
    //   throw tag0
    let mut thrower = Function::new(vec![]);
    thrower.instruction(&Instruction::I64Const(123));
    thrower.instruction(&Instruction::Throw(0));
    thrower.instruction(&Instruction::End);
    code.function(&thrower);

    // main():
    //   block (result i64)
    //     try_table () (catch tag0 0)
    //       call $thrower
    //     end
    //     unreachable
    //   end
    //   ;; stack: [i64] = 123
    //   i64.const 123
    //   i64.ne
    //   if
    //     unreachable
    //   end
    let mut main = Function::new(vec![]);
    main.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));
    main.instruction(&Instruction::TryTable(
        BlockType::Empty,
        vec![wasm_encoder::Catch::One { tag: 0, label: 0 }].into(),
    ));
    main.instruction(&Instruction::Call(0)); // call thrower
    main.instruction(&Instruction::End); // end try_table
    main.instruction(&Instruction::Unreachable);
    main.instruction(&Instruction::End); // end block
    main.instruction(&Instruction::I64Const(123));
    main.instruction(&Instruction::I64Ne);
    main.instruction(&Instruction::If(BlockType::Empty));
    main.instruction(&Instruction::Unreachable);
    main.instruction(&Instruction::End); // end if
    main.instruction(&Instruction::End); // end function
    code.function(&main);

    module.section(&code);
    module.finish()
}

fn eh_engine() -> Engine {
    let mut config = Config::new();
    config.wasm_exceptions(true);
    config.wasm_tail_call(true);
    Engine::new(&config).expect("engine with EH + tail_call")
}

fn run_wasm(engine: &Engine, wasm: &[u8]) -> Result<(), String> {
    let module = wasmtime::Module::from_binary(engine, wasm).map_err(|e| format!("load: {}", e))?;
    let mut store = Store::new(engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[])
        .map_err(|e| format!("instantiate: {}", e))?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| format!("get main: {}", e))?;
    main.call(&mut store, ())
        .map_err(|e| format!("call: {}", e))
}

#[test]
fn wasm_eh_engine_config_compiles() {
    let _engine = eh_engine();
}

#[test]
fn wasm_eh_throw_catch_roundtrip() {
    let engine = eh_engine();
    let wasm = build_throw_catch_module();
    run_wasm(&engine, &wasm).expect("throw/catch roundtrip should succeed");
}

#[test]
fn wasm_eh_uncaught_traps() {
    let engine = eh_engine();
    let wasm = build_uncaught_throw_module();
    let err = run_wasm(&engine, &wasm).expect_err("uncaught throw should trap");
    assert!(
        err.contains("uncaught")
            || err.contains("exception")
            || err.contains("unhandled")
            || err.contains("wasm trap"),
        "unexpected trap message: {}",
        err
    );
}

#[test]
fn wasm_eh_catch_all() {
    let engine = eh_engine();
    let wasm = build_catch_all_module();
    run_wasm(&engine, &wasm).expect("catch_all should succeed");
}

#[test]
fn wasm_eh_cross_function_unwind() {
    let engine = eh_engine();
    let wasm = build_cross_function_throw_catch();
    run_wasm(&engine, &wasm).expect("cross-function throw/catch should succeed");
}
