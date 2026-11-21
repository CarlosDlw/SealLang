use fixture::double;

test func integration_uses_the_public_api() {
  assert(double(21) == 42, "the integration target must see the public API");
}
