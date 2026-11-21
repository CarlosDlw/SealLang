bench func counting_loop() {
  let mut total = 0;
  let mut index = 0;
  while index < 1000 {
    total = total + index;
    index = index + 1;
  }
  assert(total == 499500);
}
