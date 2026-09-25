import { describe, test, expect } from 'bun:test';
import { EditorHistoryManager } from '../src/editor/history';
import { EditorSlide, SlideElement, defaultTransform } from '../src/editor/types';

describe('EditorHistoryManager & Batch Operation Computation', () => {
  const baseSlide: EditorSlide = {
    id: 'slide-1',
    text: 'Original lyric line',
    elements: [
      {
        type: 'TextBlock',
        id: 'tb-1',
        transform: defaultTransform(),
        block: {
          runs: [{ text: 'Original lyric line', bold: true }],
          paragraph_style: { align: 'center' },
          autofit: true
        }
      }
    ],
    speaker_notes: 'Initial note',
    ccli_metadata: {
      title: 'Amazing Grace',
      author: 'John Newton',
      ccli_number: '123456'
    }
  };

  test('maintains in-memory undo and redo stacks', () => {
    const history = new EditorHistoryManager(50);
    history.init(baseSlide);

    expect(history.canUndo()).toBe(false);
    expect(history.canRedo()).toBe(false);
    expect(history.hasChanges(baseSlide)).toBe(false);

    // Mutation 1
    const modified1: EditorSlide = JSON.parse(JSON.stringify(baseSlide));
    modified1.speaker_notes = 'Updated note 1';
    history.pushState(baseSlide);

    expect(history.canUndo()).toBe(true);
    expect(history.hasChanges(modified1)).toBe(true);

    // Mutation 2
    const modified2: EditorSlide = JSON.parse(JSON.stringify(modified1));
    modified2.speaker_notes = 'Updated note 2';
    history.pushState(modified1);

    // Undo 1 step
    const reverted1 = history.undo(modified2);
    expect(reverted1).not.toBeNull();
    expect(reverted1?.speaker_notes).toBe('Updated note 1');
    expect(history.canRedo()).toBe(true);

    // Undo 2nd step
    const reverted0 = history.undo(reverted1!);
    expect(reverted0).not.toBeNull();
    expect(reverted0?.speaker_notes).toBe('Initial note');

    // Redo 1 step
    const redone = history.redo(reverted0!);
    expect(redone).not.toBeNull();
    expect(redone?.speaker_notes).toBe('Updated note 1');

    // New mutation clears redo stack
    history.pushState(redone!);
    expect(history.canRedo()).toBe(false);
  });

  test('computes atomic batch ops on Apply for element additions and removals', () => {
    const history = new EditorHistoryManager(50);
    history.init(baseSlide);

    const editedSlide: EditorSlide = JSON.parse(JSON.stringify(baseSlide));
    // Remove tb-1
    editedSlide.elements = [];
    // Add a new Shape
    const newShape: SlideElement = {
      type: 'Shape',
      id: 'shape-1',
      transform: { x: 0.2, y: 0.2, w: 0.3, h: 0.3, rotation_deg: 0, z_index: 2, locked: false, opacity: 1.0 },
      shape_kind: 'rectangle',
      fill_color: '#ff0000'
    };
    editedSlide.elements.push(newShape);

    const ops = history.computeBatchOps(editedSlide);
    expect(ops.length).toBe(2);

    const removeOp = ops.find(o => o.op_type === 'RemoveElement');
    expect(removeOp).toBeDefined();
    expect(removeOp?.payload.element_id).toBe('tb-1');

    const addOp = ops.find(o => o.op_type === 'AddElement');
    expect(addOp).toBeDefined();
    expect(addOp?.payload.id).toBe('shape-1');
  });

  test('computes UpdateTransform and UpdateTextBlockContent for modified elements', () => {
    const history = new EditorHistoryManager(50);
    history.init(baseSlide);

    const editedSlide: EditorSlide = JSON.parse(JSON.stringify(baseSlide));
    const tb = editedSlide.elements[0] as any;
    tb.transform.x = 0.15; // moved
    tb.block.runs[0].text = 'Modified lyric line';

    const ops = history.computeBatchOps(editedSlide);
    expect(ops.some(o => o.op_type === 'UpdateTransform')).toBe(true);
    expect(ops.some(o => o.op_type === 'UpdateTextBlockContent')).toBe(true);
  });

  test('computes SetSpeakerNotes and SetCcliMetadata when notes or ccli change', () => {
    const history = new EditorHistoryManager(50);
    history.init(baseSlide);

    const editedSlide: EditorSlide = JSON.parse(JSON.stringify(baseSlide));
    editedSlide.speaker_notes = 'New pulpit instructions';
    editedSlide.ccli_metadata = {
      title: 'Amazing Grace',
      author: 'John Newton & Chris Tomlin',
      ccli_number: '789012'
    };

    const ops = history.computeBatchOps(editedSlide);
    const notesOp = ops.find(o => o.op_type === 'SetSpeakerNotes');
    expect(notesOp).toBeDefined();
    expect(notesOp?.payload.notes).toBe('New pulpit instructions');

    const ccliOp = ops.find(o => o.op_type === 'SetCcliMetadata');
    expect(ccliOp).toBeDefined();
    expect(ccliOp?.payload.metadata.author).toBe('John Newton & Chris Tomlin');
  });

  test('returns empty ops array when slide is unchanged', () => {
    const history = new EditorHistoryManager(50);
    history.init(baseSlide);

    const ops = history.computeBatchOps(baseSlide);
    expect(ops.length).toBe(0);
    expect(history.hasChanges(baseSlide)).toBe(false);
  });
});
