import assert from "node:assert/strict";
import test from "node:test";

const invoked = [];
globalThis.window = {
  __TAURI__: {
    core: {
      invoke: (cmd) => {
        invoked.push(cmd);
        return Promise.resolve(cmd);
      },
      Channel: class Channel {},
    },
  },
};
globalThis.document = { documentElement: { dataset: {} } };

test("update_self is a waiting call and tabs is not", async () => {
  await import("./transport.js");
  const transport = globalThis.localTransport;
  assert.equal(await transport.call({ cmd: "update_self" }), "machine_call_slow");
  assert.equal(await transport.call({ cmd: "tabs" }), "machine_call");
});
