// Imported by sw.js. A notification click lands here, not in the page, so this brings pinned
// forward and has it show the session the notification is about.
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    (async () => {
      const [page] = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      if (!page) return self.clients.openWindow("./");
      page.postMessage({ type: "activate-tab", tab: event.notification.data?.tab });
      return page.focus();
    })(),
  );
});
