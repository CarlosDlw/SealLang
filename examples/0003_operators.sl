func main() -> i32 {
  // arithimetic
  let a = 10 + 10;
  let b = 10 - 5;
  let c = 10 * 2;
  let d = 10 / 2;
  
  let e = 10 % 2;
  let f = -10;
  
  let g = a + b - c * d / e % 2;
  
  // Comparation
  let h = 10 == 10;
  let i = 10 != 5;
  let j = 10 < 5;
  let k = 10 > 5;
  let l = 10 <= 10;
  let m = 10 >= 10;
  
  // Logical
  let n = true && false;
  let o = true || false;
  let p = !false;
  
  // Bitwise
  let q = 12 & 10;
  let r = 12 | 10;
  let s = 12 ^ 10;
  let t = ~0;
  let u = 1 << 4;
  let v = 64 >> 2;
  
  // assignment operatos
  let mut a1 = 0;

  a1 += 10;
  a1 -= 5;
  a1 *= 2;
  a1 /= 2;
  a1 %= 2;
  
  let mut bits = 1;
  bits <<= 4;
  bits >>= 2; // too: &=, |=, ^=
  
  // increment | decrement
  let mut b1 = 10;

  b1++;
  ++b1;
  b1--;
  --b1;
  
  b1
}