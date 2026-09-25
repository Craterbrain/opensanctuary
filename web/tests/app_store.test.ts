import { test, expect, describe } from "bun:test";
import { AppStore } from "../src/core/state.ts";

describe("AppStore Reactive State Management", () => {
  test("initializes with sensible default state", () => {
    const store = new AppStore();
    const state = store.getState();
    expect(state.activeTab).toBe("songs");
    expect(state.activeCategory).toBe("all");
    expect(state.snapshot).toBeNull();
    expect(state.selectedLibraryItem).toBeNull();
    expect(state.installedBibles).toEqual([]);
  });

  test("accepts initial state overrides", () => {
    const store = new AppStore({ activeTab: "media", activeCategory: "videos" });
    const state = store.getState();
    expect(state.activeTab).toBe("media");
    expect(state.activeCategory).toBe("videos");
  });

  test("notifies subscribers upon state update", () => {
    const store = new AppStore();
    let notificationCount = 0;
    let observedTab = "";

    const unsubscribe = store.subscribe((state) => {
      notificationCount++;
      observedTab = state.activeTab;
    });

    store.setTab("scriptures", "kjv");
    expect(notificationCount).toBe(1);
    expect(observedTab).toBe("scriptures");
    expect(store.getState().activeCategory).toBe("kjv");

    unsubscribe();
    store.setTab("presentations");
    expect(notificationCount).toBe(1); // No additional call after unsubscribe
    expect(store.getState().activeTab).toBe("presentations");
  });

  test("mutates library items and selection cleanly", () => {
    const store = new AppStore();
    const mockItems = [{ id: "song-1", title: "Amazing Grace" }, { id: "song-2", title: "Way Maker" }];
    
    store.setLibraryItems(mockItems);
    expect(store.getState().activeLibraryItems.length).toBe(2);
    expect(store.getState().filteredLibraryItems.length).toBe(2);

    store.setSelectedItem(mockItems[1], 1);
    expect(store.getState().selectedLibraryItem?.id).toBe("song-2");
    expect(store.getState().selectedCatalogIndex).toBe(1);
  });
});
