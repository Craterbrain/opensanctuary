/**
 * OpenSanctuary / OS-Next Central Reactive Application Store
 * Provides typed, decoupled state management across ES modules without globalThis pollutions.
 */

export type TabType = 'songs' | 'scriptures' | 'media' | 'presentations' | 'themes';

export interface AppState {
  snapshot: any | null;
  activeTab: TabType;
  activeCategory: string;
  activeLibraryItems: any[];
  filteredLibraryItems: any[];
  selectedLibraryItem: any | null;
  selectedCatalogIndex: number;
  appOptions: Record<string, any>;
  installedBibles: any[];
  activeBibleVersion: string;
  secondaryBibleVersion: string;
  isDualBibleMode: boolean;
  expandedScheduleIndex: number | null;
  collapsedGroupIds: Set<string>;
}

export type StateListener = (state: Readonly<AppState>) => void;

export class AppStore {
  private state: AppState;
  private listeners: Set<StateListener> = new Set();

  constructor(initialState?: Partial<AppState>) {
    this.state = {
      snapshot: null,
      activeTab: 'songs',
      activeCategory: 'all',
      activeLibraryItems: [],
      filteredLibraryItems: [],
      selectedLibraryItem: null,
      selectedCatalogIndex: 0,
      appOptions: {},
      installedBibles: [],
      activeBibleVersion: 'all',
      secondaryBibleVersion: 'ASV',
      isDualBibleMode: false,
      expandedScheduleIndex: 0,
      collapsedGroupIds: new Set<string>(),
      ...initialState
    };
  }

  /**
   * Returns a shallow-immutable snapshot of the current state.
   */
  getState(): Readonly<AppState> {
    return this.state;
  }

  /**
   * Subscribes a listener to state changes. Returns an unsubscribe callback.
   */
  subscribe(listener: StateListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notify() {
    const frozenState = Object.freeze({ ...this.state });
    this.listeners.forEach(fn => {
      try {
        fn(frozenState);
      } catch (err) {
        console.error('[AppStore] Listener exception:', err);
      }
    });
  }

  /**
   * Partial state update. Merges changes and notifies listeners.
   */
  setState(partial: Partial<AppState>) {
    this.state = {
      ...this.state,
      ...partial
    };
    this.notify();
  }

  // --- Convenience typed mutators ---

  setSnapshot(snapshot: any) {
    this.setState({ snapshot });
  }

  setTab(activeTab: TabType, activeCategory = 'all') {
    this.setState({ activeTab, activeCategory });
  }

  setLibraryItems(activeLibraryItems: any[], filteredLibraryItems: any[] = activeLibraryItems) {
    this.setState({ activeLibraryItems, filteredLibraryItems });
  }

  setSelectedItem(selectedLibraryItem: any, selectedCatalogIndex = 0) {
    this.setState({ selectedLibraryItem, selectedCatalogIndex });
  }

  setOptions(appOptions: Record<string, any>) {
    this.setState({ appOptions });
  }

  setBibles(installedBibles: any[], activeBibleVersion?: string) {
    const update: Partial<AppState> = { installedBibles };
    if (activeBibleVersion !== undefined) {
      update.activeBibleVersion = activeBibleVersion;
    }
    this.setState(update);
  }
}

// Canonical singleton instance for the app
export const appStore = new AppStore();
