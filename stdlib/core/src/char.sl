module char;

// Reports whether `value` has the Unicode `White_Space` property.
//
// This is the table used by `str::trim` and `str::split_whitespace`; it is not
// a public API. The table follows the Unicode property rather than only ASCII
// whitespace.
internal func is_whitespace(value: char) -> bool {
  let code = value.to_u32();
  (code >= 0x9 && code <= 0xd)
    || code == 0x20
    || code == 0x85
    || code == 0xa0
    || code == 0x1680
    || (code >= 0x2000 && code <= 0x200a)
    || code == 0x2028
    || code == 0x2029
    || code == 0x202f
    || code == 0x205f
    || code == 0x3000
}
