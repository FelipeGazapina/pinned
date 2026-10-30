import assert from "node:assert/strict";
import test from "node:test";
import { createUploadImages } from "./upload-images.js";

const key = "mac\n/uploads/shot.png";

test("a missing upload drops the button and is not read again", async () => {
  let calls = 0;
  const images = createUploadImages();
  const read = () => {
    calls += 1;
    return Promise.reject("could not read /uploads/shot.png: No such file or directory");
  };
  assert.equal(await images.pull(key, read), "drop");
  assert.equal(await images.pull(key, read), "drop");
  images.reconnect();
  assert.equal(await images.pull(key, () => Promise.resolve("data:image/png;base64,aaa")), "drop");
  assert.equal(calls, 1);
});

test("a file the machine refuses stays dropped", async () => {
  const images = createUploadImages();
  for (const message of [
    "/uploads/notes.txt is not an upload",
    "/uploads/notes.txt is not a picture",
    "/uploads/huge.png is over 8 MB",
    "this machine keeps no files",
  ]) {
    const id = `mac\n${message}`;
    let calls = 0;
    const read = () => {
      calls += 1;
      return Promise.reject(message);
    };
    assert.equal(await images.pull(id, read), "drop");
    assert.equal(await images.pull(id, read), "drop");
    assert.equal(calls, 1);
  }
});

test("a hub disconnect keeps the button and reads again after reconnect", async () => {
  let calls = 0;
  let down = true;
  const images = createUploadImages();
  const read = () => {
    calls += 1;
    return down ? Promise.reject("lost the connection to the hub") : Promise.resolve("data:image/png;base64,aaa");
  };
  assert.equal(await images.pull(key, read), "wait");
  assert.equal(await images.pull(key, read), "wait");
  assert.equal(calls, 1);
  assert.deepEqual(images.reconnect(), [key]);
  down = false;
  assert.equal(await images.pull(key, read), "data:image/png;base64,aaa");
  assert.equal(await images.pull(key, read), "data:image/png;base64,aaa");
  assert.equal(calls, 2);
});

test("an empty read drops the button", async () => {
  let calls = 0;
  const images = createUploadImages();
  const read = () => {
    calls += 1;
    return Promise.resolve(null);
  };
  assert.equal(await images.pull(key, read), "drop");
  assert.equal(await images.pull(key, read), "drop");
  assert.equal(calls, 1);
});

test("a disconnect that arrives after reconnect is read again", async () => {
  let calls = 0;
  let rejectRead;
  const images = createUploadImages();
  const read = () => {
    calls += 1;
    if (calls === 1) return new Promise((_, reject) => { rejectRead = reject; });
    return Promise.resolve("data:image/png;base64,aaa");
  };
  const first = images.pull(key, read);
  await Promise.resolve();
  assert.deepEqual(images.reconnect(), []);
  rejectRead("lost the connection to the hub");
  assert.equal(await first, "retry");
  assert.equal(await images.pull(key, read), "data:image/png;base64,aaa");
  assert.equal(calls, 2);
});

test("an offline machine waits instead of dropping the button", async () => {
  const images = createUploadImages();
  for (const message of [
    "not connected to the hub",
    "mac-mini is offline",
    "mac-mini went offline",
    "mac-mini did not answer",
  ]) {
    let calls = 0;
    const id = `mac\n${message}`;
    const read = () => {
      calls += 1;
      return Promise.reject(message);
    };
    assert.equal(await images.pull(id, read), "wait");
    assert.equal(await images.pull(id, read), "wait");
    assert.equal(calls, 1);
    assert.deepEqual(images.reconnect(), [id]);
    assert.equal(await images.pull(id, () => Promise.resolve("data:image/png;base64,bbb")), "data:image/png;base64,bbb");
  }
});
