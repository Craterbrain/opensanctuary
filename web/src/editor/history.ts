import { EditorSlide, SlideEditOp } from './types';

export class EditorHistoryManager {
  private undoStack: EditorSlide[] = [];
  private redoStack: EditorSlide[] = [];
  private initialSnapshot: EditorSlide | null = null;
  private maxHistory: number = 100;

  constructor(maxHistory: number = 100) {
    this.maxHistory = maxHistory;
  }

  public init(initialSlide: EditorSlide): void {
    const clone = JSON.parse(JSON.stringify(initialSlide));
    this.initialSnapshot = clone;
    this.undoStack = [];
    this.redoStack = [];
  }

  public pushState(slide: EditorSlide): void {
    const clone = JSON.parse(JSON.stringify(slide));
    this.undoStack.push(clone);
    if (this.undoStack.length > this.maxHistory) {
      this.undoStack.shift();
    }
    this.redoStack = []; // clear redo on new mutation
  }

  public canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  public canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  public undo(currentSlide: EditorSlide): EditorSlide | null {
    if (!this.canUndo()) return null;
    const prev = this.undoStack.pop()!;
    this.redoStack.push(JSON.parse(JSON.stringify(currentSlide)));
    return JSON.parse(JSON.stringify(prev));
  }

  public redo(currentSlide: EditorSlide): EditorSlide | null {
    if (!this.canRedo()) return null;
    const next = this.redoStack.pop()!;
    this.undoStack.push(JSON.parse(JSON.stringify(currentSlide)));
    return JSON.parse(JSON.stringify(next));
  }

  public getInitialSnapshot(): EditorSlide | null {
    return this.initialSnapshot;
  }

  public hasChanges(currentSlide: EditorSlide): boolean {
    if (!this.initialSnapshot) return false;
    return JSON.stringify(this.initialSnapshot) !== JSON.stringify(currentSlide);
  }

  /**
   * Produce the atomic batch ops required by BatchSlideEdit on Apply.
   * Invariant: inv.slide.batch-undo
   */
  public computeBatchOps(currentSlide: EditorSlide): SlideEditOp[] {
    const initial = this.initialSnapshot;
    const ops: SlideEditOp[] = [];

    if (!initial) {
      // If no initial snapshot, replace everything
      for (const el of currentSlide.elements) {
        ops.push({ op_type: 'AddElement', payload: el });
      }
      if (currentSlide.speaker_notes) {
        ops.push({ op_type: 'SetSpeakerNotes', payload: { notes: currentSlide.speaker_notes } });
      }
      if (currentSlide.background_v2) {
        ops.push({ op_type: 'SetBackground', payload: { background: currentSlide.background_v2 } });
      }
      if (currentSlide.transition) {
        ops.push({ op_type: 'SetTransition', payload: { transition: currentSlide.transition } });
      }
      return ops;
    }

    const initMap = new Map(initial.elements.map(e => [e.id, e]));
    const curMap = new Map(currentSlide.elements.map(e => [e.id, e]));

    // 1. Removed elements
    for (const [id] of initMap) {
      if (!curMap.has(id)) {
        ops.push({ op_type: 'RemoveElement', payload: { element_id: id } });
      }
    }

    // 2. Added or modified elements
    for (const el of currentSlide.elements) {
      const prev = initMap.get(el.id);
      if (!prev) {
        ops.push({ op_type: 'AddElement', payload: el });
      } else {
        // Check transform change
        if (JSON.stringify(prev.transform) !== JSON.stringify(el.transform)) {
          ops.push({
            op_type: 'UpdateTransform',
            payload: { element_id: el.id, transform: el.transform }
          });
        }
        // Check text content change
        if (el.type === 'TextBlock' && prev.type === 'TextBlock') {
          if (JSON.stringify(prev.block.runs) !== JSON.stringify(el.block.runs) ||
              JSON.stringify(prev.block.paragraph_style) !== JSON.stringify(el.block.paragraph_style)) {
            ops.push({
              op_type: 'UpdateTextBlockContent',
              payload: {
                element_id: el.id,
                runs: el.block.runs,
                paragraph_style: el.block.paragraph_style
              }
            });
          }
          if (JSON.stringify(prev.block.effects) !== JSON.stringify(el.block.effects)) {
            ops.push({
              op_type: 'UpdateElementEffects',
              payload: { element_id: el.id, effects: el.block.effects }
            });
          }
        }
      }
    }

    // 3. Speaker notes
    if (initial.speaker_notes !== currentSlide.speaker_notes) {
      ops.push({
        op_type: 'SetSpeakerNotes',
        payload: { notes: currentSlide.speaker_notes || '' }
      });
    }

    // 4. CCLI metadata
    if (JSON.stringify(initial.ccli_metadata) !== JSON.stringify(currentSlide.ccli_metadata)) {
      ops.push({
        op_type: 'SetCcliMetadata',
        payload: { metadata: currentSlide.ccli_metadata || {} }
      });
    }

    // 5. Background
    if (JSON.stringify(initial.background_v2) !== JSON.stringify(currentSlide.background_v2) && currentSlide.background_v2) {
      ops.push({
        op_type: 'SetBackground',
        payload: { background: currentSlide.background_v2 }
      });
    }

    // 6. Transition
    if (JSON.stringify(initial.transition) !== JSON.stringify(currentSlide.transition) && currentSlide.transition) {
      ops.push({
        op_type: 'SetTransition',
        payload: { transition: currentSlide.transition }
      });
    }

    return ops;
  }
}
