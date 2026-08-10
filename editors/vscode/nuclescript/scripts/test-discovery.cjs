const assert = require("node:assert/strict");
const test = require("node:test");
const { discoverTests } = require("../out/testDiscovery");

test("discovers top-level tests in source order", () => {
  const source = `
test "stores a file" { assert true }
test "recovers data" { assert true }
`;
  assert.deepEqual(
    discoverTests(source).map(({ name }) => name),
    ["stores a file", "recovers data"],
  );
});

test("ignores test-shaped text in strings, comments, and nested blocks", () => {
  const source = `
let prose: Str = "test \\"not real\\" {}"
// test "also not real" {}
fn helper() returns Void {
  test "nested and not runnable" {}
}
test "real" { assert true }
`;
  assert.deepEqual(discoverTests(source).map(({ name }) => name), ["real"]);
});

test("decodes escaped test names and skips unterminated names", () => {
  const source = `test "quote: \\"yes\\"" {}
test "unterminated`;
  const discovered = discoverTests(source);
  assert.equal(discovered.length, 1);
  assert.equal(discovered[0].name, 'quote: "yes"');
  assert.equal(source.slice(discovered[0].startOffset, discovered[0].endOffset), 'test "quote: \\"yes\\""');
});
