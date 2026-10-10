/**
 * OpenSanctuary / OS-Next Song Arrangement Editor Modal Controller
 * Extracted from app_core.ts (part of the ongoing modularization pass) — the
 * verse/chorus/bridge play-order editor: a master-sections list (the song's
 * authored slides) and a play-order queue (the arrangement actually performed),
 * built with drag-and-drop between and within both lists.
 */
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';
import { type ArrangementEntry, escapeUserHtml, isPrimaryDropTarget } from '../core/presentation_helpers';
import type { ToastType } from '../core/ui_utils';

export interface ArrangementModalContext {
  showToast: (message: string, type?: ToastType) => void;
  showModal: (el: HTMLElement) => void;
  closeModal: (el: HTMLElement) => void;
  sendCommand: (cmd: any) => void;
  getSlideBadge: (slide: any, idx?: number) => string;
  getSlideBadgeColor: (badge: string) => string;
  escapeHtml: (str: string) => string;
  formatCssBackground: (bg: string | null | undefined) => string;
  getCurrentSnapshot: () => any;
  renderSchedule: (schedule: any) => void;
  renderPreviewDeck: (state: any) => void;
  renderLiveDeck: (state: any) => void;
}

let ctx: ArrangementModalContext | null = null;

export function initArrangementModal(context: ArrangementModalContext) {
  ctx = context;

  const arrangementModal = document.getElementById('arrangement-modal');

  document.getElementById('btn-arrangement-reset')?.addEventListener('click', () => {
    localArrangement = localMasterSlides.map((s, idx) => ({
      section_id: s.badge,
      source_slide_index: idx,
      background_override: null,
    }));
    renderArrangementModalPanes();
    ctx!.showToast('Reset arrangement to author order', 'info');
  });

  document.getElementById('btn-cancel-arrangement')?.addEventListener('click', () => {
    if (arrangementModal) ctx!.closeModal(arrangementModal);
  });
  document.getElementById('btn-close-arrangement')?.addEventListener('click', () => {
    if (arrangementModal) ctx!.closeModal(arrangementModal);
  });

  document.getElementById('btn-apply-arrangement')?.addEventListener('click', () => {
    if (activeArrangementItemIndex !== null && activeArrangementTargetItem) {
      if (localArrangement.length === 0) {
        ctx!.showToast('Cannot apply an empty arrangement', 'warning');
        return;
      }
      ctx!.sendCommand({
        SetArrangement: {
          item_index: activeArrangementItemIndex,
          arrangement: localArrangement,
        }
      });
      activeArrangementTargetItem.arrangement = localArrangement.slice();
      const snapshot = ctx!.getCurrentSnapshot();
      if (snapshot && snapshot.schedule) {
        ctx!.renderSchedule(snapshot.schedule);
      }
      if (snapshot && snapshot.state) {
        ctx!.renderPreviewDeck(snapshot.state);
        ctx!.renderLiveDeck(snapshot.state);
      }
      ctx!.showToast(`✓ Applied arrangement (${localArrangement.length} sections) to "${activeArrangementTargetItem.title}"`, 'success');
    }
    if (arrangementModal) ctx!.closeModal(arrangementModal);
  });

  // Modal-scoped shortcuts: (↑, ↓, Enter, Delete, Ctrl+D, Ctrl+↑/↓, Esc, Ctrl+Enter)
  window.addEventListener('keydown', (e: KeyboardEvent) => {
    if (!arrangementModal || arrangementModal.style.display === 'none') return;

    // Esc: Close/cancel without dispatch
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      ctx!.closeModal(arrangementModal);
      return;
    }

    // Ctrl+Enter: Apply arrangement
    if (e.ctrlKey && e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      const applyBtn = document.getElementById('btn-apply-arrangement');
      if (applyBtn) applyBtn.click();
      return;
    }

    // Ctrl+Up: Move selected play entry up
    if (e.ctrlKey && e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedPlayIndex !== null && selectedPlayIndex > 0) {
        const [moved] = localArrangement.splice(selectedPlayIndex, 1);
        localArrangement.splice(selectedPlayIndex - 1, 0, moved);
        selectedPlayIndex--;
        renderArrangementModalPanes();
      }
      return;
    }

    // Ctrl+Down: Move selected play entry down
    if (e.ctrlKey && e.key === 'ArrowDown') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedPlayIndex !== null && selectedPlayIndex < localArrangement.length - 1) {
        const [moved] = localArrangement.splice(selectedPlayIndex, 1);
        localArrangement.splice(selectedPlayIndex + 1, 0, moved);
        selectedPlayIndex++;
        renderArrangementModalPanes();
      }
      return;
    }

    // Ctrl+D: Duplicate selected play entry
    if (e.ctrlKey && e.key.toLowerCase() === 'd') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedPlayIndex !== null && selectedPlayIndex < localArrangement.length) {
        const dup = JSON.parse(JSON.stringify(localArrangement[selectedPlayIndex]));
        localArrangement.splice(selectedPlayIndex + 1, 0, dup);
        selectedPlayIndex++;
        renderArrangementModalPanes();
      }
      return;
    }

    // ArrowUp: Navigate play entry selection up
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedPlayIndex !== null && selectedPlayIndex > 0) {
        selectedPlayIndex--;
      } else if (localArrangement.length > 0) {
        selectedPlayIndex = localArrangement.length - 1;
      }
      renderArrangementModalPanes();
      return;
    }

    // ArrowDown: Navigate play entry selection down
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedPlayIndex !== null && selectedPlayIndex < localArrangement.length - 1) {
        selectedPlayIndex++;
      } else if (localArrangement.length > 0) {
        selectedPlayIndex = 0;
      }
      renderArrangementModalPanes();
      return;
    }

    // Delete or Backspace: Remove selected play entry
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedPlayIndex !== null && selectedPlayIndex < localArrangement.length) {
        localArrangement.splice(selectedPlayIndex, 1);
        if (selectedPlayIndex >= localArrangement.length) {
          selectedPlayIndex = Math.max(0, localArrangement.length - 1);
        }
        renderArrangementModalPanes();
      }
      return;
    }

    // Enter: Add selected master section to play order
    if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      if (selectedMasterIndex !== null && localMasterSlides[selectedMasterIndex]) {
        const m = localMasterSlides[selectedMasterIndex];
        localArrangement.push({
          section_id: m.badge,
          source_slide_index: selectedMasterIndex,
          background_override: null,
        });
        selectedPlayIndex = localArrangement.length - 1;
        renderArrangementModalPanes();
      }
      return;
    }
  });
}

let activeArrangementItemIndex: number | null = null;
let activeArrangementTargetItem: any = null;
let localMasterSlides: any[] = [];
let localArrangement: ArrangementEntry[] = [];
let arrangementDndCleanups: Array<() => void> = [];
let selectedPlayIndex: number | null = null;
let selectedMasterIndex: number | null = null;

export function openArrangementModal(itemIndex: number, item: any) {
  if (!item || !item.slides || item.slides.length === 0) {
    ctx!.showToast('Item has no slides to arrange', 'info');
    return;
  }
  activeArrangementItemIndex = itemIndex;
  activeArrangementTargetItem = item;

  // Extract master slides with badges
  localMasterSlides = item.slides.map((s: any, idx: number) => ({
    ...s,
    masterIndex: idx,
    badge: ctx!.getSlideBadge(s, idx),
  }));

  // Initialize or clone arrangement
  if (item.arrangement && Array.isArray(item.arrangement) && item.arrangement.length > 0) {
    localArrangement = JSON.parse(JSON.stringify(item.arrangement));
  } else {
    localArrangement = localMasterSlides.map((s, idx) => ({
      section_id: s.badge,
      source_slide_index: idx,
      background_override: null,
    }));
  }

  selectedPlayIndex = localArrangement.length > 0 ? 0 : null;
  selectedMasterIndex = null;

  const titleEl = document.getElementById('arrangement-modal-title');
  if (titleEl) {
    titleEl.innerHTML = `🎼 Song Arrangement — ${escapeUserHtml(item.title || 'Song')}`;
  }

  document.body.classList.add('editor-open');
  renderArrangementModalPanes();
  const arrangementModal = document.getElementById('arrangement-modal');
  if (arrangementModal) ctx!.showModal(arrangementModal);
}

function updateArrangementPreview(masterSlide: any, backgroundOverride: string | null = null) {
  const previewTextEl = document.getElementById('arrangement-preview-text');
  const previewBgEl = document.getElementById('arrangement-preview-bg');
  const badgeLabelEl = document.getElementById('arrangement-preview-badge-label');

  if (!masterSlide) return;

  if (previewTextEl) {
    previewTextEl.textContent = masterSlide.text || '';
  }
  if (badgeLabelEl) {
    badgeLabelEl.textContent = `[${masterSlide.badge}] ${masterSlide.header || masterSlide.label || ''}`;
  }
  if (previewBgEl) {
    const effectiveBg = backgroundOverride || masterSlide.background || (activeArrangementTargetItem ? activeArrangementTargetItem.background : null);
    if (effectiveBg) {
      previewBgEl.style.background = ctx!.formatCssBackground(effectiveBg);
    } else {
      previewBgEl.style.background = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)';
    }
  }
}

function renderArrangementModalPanes() {
  arrangementDndCleanups.forEach(fn => { try { fn(); } catch (_) {} });
  arrangementDndCleanups = [];

  const masterListEl = document.getElementById('arrangement-master-list');
  const playListEl = document.getElementById('arrangement-play-list');
  const playCountEl = document.getElementById('arrangement-play-count');

  if (playCountEl) {
    playCountEl.textContent = `${localArrangement.length} section${localArrangement.length === 1 ? '' : 's'}`;
  }

  // Preview first item if available
  if (localArrangement.length > 0) {
    const firstEntry = localArrangement[0];
    const m = localMasterSlides[firstEntry.source_slide_index] || localMasterSlides[0];
    updateArrangementPreview(m, firstEntry.background_override);
  } else if (localMasterSlides.length > 0) {
    updateArrangementPreview(localMasterSlides[0], null);
  }

  // 1. Master Sections List
  if (masterListEl) {
    masterListEl.innerHTML = '';
    localMasterSlides.forEach((master, mIdx) => {
      const card = document.createElement('div');
      card.className = 'arrangement-master-card';
      const isMasterSelected = mIdx === selectedMasterIndex;
      card.style.cssText = `
        padding: 8px 10px;
        background: ${isMasterSelected ? 'rgba(255, 167, 38, 0.12)' : 'rgba(255, 255, 255, 0.05)'};
        border: 1px solid ${isMasterSelected ? 'var(--os-brand-amber, #ffa726)' : 'var(--os-border, #2d3139)'};
        border-radius: 6px;
        display: flex;
        align-items: center;
        gap: 8px;
        cursor: pointer;
        user-select: none;
        transition: all 0.15s ease;
      `;
      const badgeColor = ctx!.getSlideBadgeColor(master.badge);
      const firstLine = (master.text || '').split('\n')[0] || '(empty text)';

      card.innerHTML = `
        <span style="font-weight: 700; font-size: 11px; color: ${badgeColor}; width: 28px;">${ctx!.escapeHtml(master.badge)}</span>
        <span style="flex: 1; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-main, #eee);">${ctx!.escapeHtml(firstLine)}</span>
        <button class="btn btn-tag btn-add-section" title="Append to play order" style="padding: 2px 6px; font-size: 10px; border-radius: 4px; background: rgba(255,167,38,0.15); color: var(--os-brand-amber, #ffa726); border: 1px solid var(--os-brand-amber, #ffa726); cursor: pointer;">+ Add</button>
      `;

      card.addEventListener('mouseenter', () => updateArrangementPreview(master, null));
      card.addEventListener('click', () => {
        selectedMasterIndex = mIdx;
        selectedPlayIndex = null;
        updateArrangementPreview(master, null);
        renderArrangementModalPanes();
      });

      const addBtn = card.querySelector('.btn-add-section');
      if (addBtn) {
        addBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          localArrangement.push({
            section_id: master.badge,
            source_slide_index: mIdx,
            background_override: null,
          });
          selectedPlayIndex = localArrangement.length - 1;
          selectedMasterIndex = null;
          renderArrangementModalPanes();
        });
      }

      arrangementDndCleanups.push(
        draggable({
          element: card,
          getInitialData: () => ({ type: 'master-section', masterIndex: mIdx, badge: master.badge }),
          onDragStart: () => card.style.opacity = '0.5',
          onDrop: () => card.style.opacity = '1',
        })
      );

      masterListEl.appendChild(card);
    });
  }

  // 2. Play Order Queue
  if (playListEl) {
    playListEl.innerHTML = '';
    if (localArrangement.length === 0) {
      playListEl.innerHTML = `
        <div style="padding: 24px; text-align: center; color: var(--text-muted, #777); font-size: 11px;">
          No sections in play order. Click <b>+ Add</b> on master sections to build the song progression.
        </div>
      `;
    } else {
      localArrangement.forEach((entry, pIdx) => {
        const master = localMasterSlides[entry.source_slide_index] || localMasterSlides[0];
        const isPlaySelected = pIdx === selectedPlayIndex;
        const card = document.createElement('div');
        card.className = `arrangement-play-card ${isPlaySelected ? 'selected-play-card' : ''}`;
        card.style.cssText = `
          padding: 6px 10px;
          background: ${isPlaySelected ? 'rgba(255, 167, 38, 0.12)' : 'rgba(255, 255, 255, 0.04)'};
          border: 1px solid ${isPlaySelected ? 'var(--os-brand-amber, #ffa726)' : 'var(--os-border, #2d3139)'};
          border-radius: 6px;
          display: flex;
          align-items: center;
          gap: 8px;
          user-select: none;
          cursor: pointer;
        `;
        const badgeColor = ctx!.getSlideBadgeColor(entry.section_id);
        const firstLine = (master?.text || '').split('\n')[0] || '';
        const hasBgOverride = !!entry.background_override;

        card.innerHTML = `
          <span class="arr-play-drag-handle drag-handle" title="Drag to reorder" style="cursor: grab; color: var(--text-dim, #777); font-size: 12px;">⠿</span>
          <span style="font-size: 10px; color: var(--text-muted, #888); width: 18px;">${pIdx + 1}.</span>
          <span style="font-weight: 700; font-size: 11px; color: ${badgeColor}; width: 28px;">${ctx!.escapeHtml(entry.section_id)}</span>
          <span style="flex: 1; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-main, #eee);">${ctx!.escapeHtml(firstLine)}</span>
          ${hasBgOverride ? '<span title="Custom per-play background override" style="font-size: 10px;">🖼️</span>' : ''}
          <button class="btn-remove-entry" title="Remove from play order" style="background: none; border: none; color: #ff8888; cursor: pointer; font-size: 12px; padding: 0 4px; line-height: 1;">✕</button>
        `;

        card.addEventListener('mouseenter', () => updateArrangementPreview(master, entry.background_override));
        card.addEventListener('click', (e) => {
          e.stopPropagation();
          selectedPlayIndex = pIdx;
          selectedMasterIndex = null;
          updateArrangementPreview(master, entry.background_override);
          renderArrangementModalPanes();
        });

        const removeBtn = card.querySelector('.btn-remove-entry');
        if (removeBtn) {
          removeBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            localArrangement.splice(pIdx, 1);
            if (selectedPlayIndex !== null && selectedPlayIndex >= localArrangement.length) {
              selectedPlayIndex = Math.max(0, localArrangement.length - 1);
            }
            renderArrangementModalPanes();
          });
        }

        const handle = card.querySelector<HTMLElement>('.arr-play-drag-handle');
        if (handle) {
          arrangementDndCleanups.push(
            draggable({
              element: card,
              dragHandle: handle,
              getInitialData: () => ({ type: 'play-entry', playIndex: pIdx }),
              onDragStart: () => card.style.opacity = '0.5',
              onDrop: () => card.style.opacity = '1',
            })
          );
        }

        arrangementDndCleanups.push(
          dropTargetForElements({
            element: card,
            getData: () => ({ type: 'play-entry-target', targetIndex: pIdx }),
            onDragEnter: () => card.style.borderColor = 'var(--os-brand-amber, #ffa726)',
            onDragLeave: () => card.style.borderColor = 'var(--os-border, #2d3139)',
            onDrop: ({ source }) => {
              card.style.borderColor = 'var(--os-border, #2d3139)';
              const data = source.data;
              if (data.type === 'master-section') {
                const mIdx = data.masterIndex as number;
                const mSlide = localMasterSlides[mIdx];
                if (mSlide) {
                  localArrangement.splice(pIdx, 0, {
                    section_id: mSlide.badge,
                    source_slide_index: mIdx,
                    background_override: null,
                  });
                  renderArrangementModalPanes();
                }
              } else if (data.type === 'play-entry') {
                const fromIdx = data.playIndex as number;
                if (fromIdx !== pIdx && fromIdx >= 0 && fromIdx < localArrangement.length) {
                  const entry = localArrangement.splice(fromIdx, 1)[0];
                  localArrangement.splice(pIdx, 0, entry);
                  renderArrangementModalPanes();
                }
              }
            }
          })
        );

        playListEl.appendChild(card);
      });
    }

    arrangementDndCleanups.push(
      dropTargetForElements({
        element: playListEl,
        getData: () => ({ type: 'play-list-container' }),
        onDrop: ({ source, location }) => {
          // A card's own drop target already handled this drop if it was
          // the innermost target dropped on -- without this guard, dropping
          // on a card fires both the card's insert-at-position handler AND
          // this container's append-to-end handler, adding the section twice.
          if (!isPrimaryDropTarget(playListEl, location.current.dropTargets)) {
            return;
          }
          const data = source.data;
          if (data.type === 'master-section') {
            const mIdx = data.masterIndex as number;
            const mSlide = localMasterSlides[mIdx];
            if (mSlide) {
              localArrangement.push({
                section_id: mSlide.badge,
                source_slide_index: mIdx,
                background_override: null,
              });
              renderArrangementModalPanes();
            }
          }
        }
      })
    );
  }
}
