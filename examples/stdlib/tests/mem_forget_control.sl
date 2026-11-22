// `mem::forget` in branches, loops, temporaries, and early returns must clear
// the right drop flags. Every test uses its own destructor counter so the
// cases stay independent.

use std::mem::forget;

struct BranchToken {
  value: i32,
}

struct TempToken {
  value: i32,
}

struct LoopToken {
  value: i32,
}

struct ReturnToken {
  value: i32,
}

struct ConditionalToken {
  value: i32,
}

static mut BRANCH_DROPS: i32 = 0;
static mut TEMP_DROPS: i32 = 0;
static mut LOOP_DROPS: i32 = 0;
static mut RETURN_DROPS: i32 = 0;
static mut CONDITIONAL_DROPS: i32 = 0;

impl Drop for BranchToken {
  func drop(self: *mut BranchToken) {
    unsafe {
      BRANCH_DROPS += 1;
    }
  }
}

impl Drop for TempToken {
  func drop(self: *mut TempToken) {
    unsafe {
      TEMP_DROPS += 1;
    }
  }
}

impl Drop for LoopToken {
  func drop(self: *mut LoopToken) {
    unsafe {
      LOOP_DROPS += 1;
    }
  }
}

impl Drop for ReturnToken {
  func drop(self: *mut ReturnToken) {
    unsafe {
      RETURN_DROPS += 1;
    }
  }
}

impl Drop for ConditionalToken {
  func drop(self: *mut ConditionalToken) {
    unsafe {
      CONDITIONAL_DROPS += 1;
    }
  }
}

func consume_branch(flag: bool) {
  let token = BranchToken { value: 1 };
  if flag {
    forget(token);
  } else {
    drop(token);
  }
}

func forget_temporary() {
  forget(TempToken { value: 2 });
}

func forget_in_loop() {
  let mut iteration = 0;
  while iteration < 3 {
    let token = LoopToken { value: iteration };
    forget(token);
    iteration += 1;
  }
}

func consume_conditional(flag: bool) {
  let token: ConditionalToken;
  if flag {
    token = ConditionalToken { value: 4 };
    forget(token);
  } else {
    token = ConditionalToken { value: 5 };
    drop(token);
  }
}

func forget_then_return(flag: bool) -> i32 {
  let token = ReturnToken { value: 3 };
  if flag {
    forget(token);
    return 1;
  }
  drop(token);
  2
}

test func forget_in_one_branch_and_drop_in_the_other() {
  consume_branch(true);
  assert(unsafe { BRANCH_DROPS } == 0);
  consume_branch(false);
  assert(unsafe { BRANCH_DROPS } == 1);
}

test func forgetting_a_temporary_suppresses_its_scope_drop() {
  forget_temporary();
  assert(unsafe { TEMP_DROPS } == 0);
}

test func forgetting_inside_a_loop_suppresses_every_iteration() {
  forget_in_loop();
  assert(unsafe { LOOP_DROPS } == 0);
}

test func conditional_initialization_clears_only_the_forget_path() {
  consume_conditional(true);
  assert(unsafe { CONDITIONAL_DROPS } == 0);
  consume_conditional(false);
  assert(unsafe { CONDITIONAL_DROPS } == 1);
}

test func forgetting_before_an_early_return_clears_the_exit_drop() {
  assert(forget_then_return(true) == 1);
  assert(unsafe { RETURN_DROPS } == 0);
  assert(forget_then_return(false) == 2);
  assert(unsafe { RETURN_DROPS } == 1);
}
