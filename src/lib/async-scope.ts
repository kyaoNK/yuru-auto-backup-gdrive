/** Own async subscriptions and reject stale/unmounted responses. No framework dependency. */
export function createAsyncScope() {
  let active = true;
  const cleanup = new Set<() => void>();
  const versions = new Map<string, number>();
  return {
    get active() { return active; },
    ticket(key: string) {
      const version = (versions.get(key) ?? 0) + 1;
      versions.set(key, version);
      return () => active && versions.get(key) === version;
    },
    async own(subscription: Promise<() => void>, onError: (error: unknown) => void) {
      try {
        const stop = await subscription;
        const release = () => { void Promise.resolve().then(stop).catch(error => { if (active) onError(error); }); };
        if (active) cleanup.add(release);
        else release();
      } catch (error) { if (active) onError(error); }
    },
    dispose() {
      active = false;
      for (const stop of cleanup) stop();
      cleanup.clear();
    },
  };
}
