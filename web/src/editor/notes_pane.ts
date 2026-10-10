import { EditorSlide, CcliMetadata } from './types';

export class NotesPane {
  private containerEl: HTMLElement;
  private currentSlide: EditorSlide | null = null;
  private onChangeCb: ((slide: EditorSlide) => void) | null = null;

  private notesInput!: HTMLTextAreaElement;
  private ccliTitleInput!: HTMLInputElement;
  private ccliAuthorInput!: HTMLInputElement;
  private ccliCopyrightInput!: HTMLInputElement;
  private ccliNumInput!: HTMLInputElement;
  private bodyEl!: HTMLElement;

  constructor(containerEl: HTMLElement) {
    this.containerEl = containerEl;
    this.render();
  }

  public bind(slide: EditorSlide): void {
    this.currentSlide = slide;
    if (this.notesInput) {
      this.notesInput.value = slide.speaker_notes || slide.notes || '';
    }
    const ccli = slide.ccli_metadata || {};
    if (this.ccliTitleInput) this.ccliTitleInput.value = ccli.title || '';
    if (this.ccliAuthorInput) this.ccliAuthorInput.value = ccli.author || '';
    if (this.ccliCopyrightInput) this.ccliCopyrightInput.value = ccli.copyright || '';
    if (this.ccliNumInput) this.ccliNumInput.value = ccli.ccli_number || '';
  }

  public onChange(cb: (slide: EditorSlide) => void): void {
    this.onChangeCb = cb;
  }

  private render(): void {
    // [ARCH:slide-editor-shell] A leftover CSS class fights live-JS sizing
    // here if it's ever reintroduced.
    // Note: className is 'editor-notes-pane' only — a stray legacy
    // 'studio-notes-drawer' class used to also get applied here, which
    // carries its own `max-height: 120px` rule (style.css) left over from an
    // older drawer-style design. That cap fought this element's real sizing
    // (a normal `flex: 0 0 auto` column child that should size to its actual
    // content), silently truncating it and spilling its bottom content past
    // the shell's `overflow: hidden` boundary — which lines up exactly with
    // the footer above it, making clipped content look like it was hidden
    // behind the Save bar. There's no separate collapse state any more: the
    // shell's flex layout (middleRegion has `flex: 1 1 0%; min-height: 0`)
    // already guarantees this pane always gets exactly the space it needs
    // between the toolbar and the footer, shrinking the canvas/properties
    // area instead of ever overflowing.
    this.containerEl.className = 'editor-notes-pane';

    // Header bar
    const header = document.createElement('div');
    header.className = 'studio-notes-header';

    const titleSpan = document.createElement('span');
    titleSpan.className = 'studio-notes-title';
    titleSpan.innerHTML = '📝 <span>Speaker Notes &amp; CCLI Metadata (Stage Monitor &amp; Broadcast)</span>';

    header.appendChild(titleSpan);
    this.containerEl.appendChild(header);

    // Body container: 2 columns (Speaker notes left, CCLI metadata right)
    this.bodyEl = document.createElement('div');
    this.bodyEl.className = 'studio-notes-body';

    // Left column: Notes
    const leftCol = document.createElement('div');
    leftCol.className = 'studio-notes-col';

    this.notesInput = document.createElement('textarea');
    this.notesInput.className = 'search-input studio-notes-input';
    this.notesInput.placeholder = 'Add speaker cues, scripture references, or stage prompts for this slide...';
    this.notesInput.addEventListener('input', () => {
      if (this.currentSlide) {
        this.currentSlide.speaker_notes = this.notesInput.value;
        this.currentSlide.notes = this.notesInput.value;
        this.notifyChange();
      }
    });

    leftCol.appendChild(this.notesInput);
    this.bodyEl.appendChild(leftCol);

    // Right column: CCLI metadata fields
    const rightCol = document.createElement('div');
    rightCol.className = 'studio-notes-meta';

    this.ccliTitleInput = this.createMetaInput('Song/Piece Title', (val) => {
      this.ensureCcli().title = val;
      this.notifyChange();
    });
    this.ccliAuthorInput = this.createMetaInput('Author / Composer', (val) => {
      this.ensureCcli().author = val;
      this.notifyChange();
    });
    this.ccliCopyrightInput = this.createMetaInput('Copyright Info', (val) => {
      this.ensureCcli().copyright = val;
      this.notifyChange();
    });
    this.ccliNumInput = this.createMetaInput('CCLI Song #', (val) => {
      this.ensureCcli().ccli_number = val;
      this.notifyChange();
    });

    rightCol.appendChild(this.wrapInput('Title:', this.ccliTitleInput));
    rightCol.appendChild(this.wrapInput('Author:', this.ccliAuthorInput));
    rightCol.appendChild(this.wrapInput('Copyright:', this.ccliCopyrightInput));
    rightCol.appendChild(this.wrapInput('CCLI #:', this.ccliNumInput));
    this.bodyEl.appendChild(rightCol);

    this.containerEl.appendChild(this.bodyEl);
  }

  private wrapInput(label: string, input: HTMLInputElement): HTMLElement {
    const wrap = document.createElement('div');
    wrap.className = 'studio-notes-field';
    const lbl = document.createElement('label');
    lbl.textContent = label;
    wrap.appendChild(lbl);
    wrap.appendChild(input);
    return wrap;
  }

  private createMetaInput(placeholder: string, onUpdate: (val: string) => void): HTMLInputElement {
    const inp = document.createElement('input');
    inp.type = 'text';
    inp.className = 'search-input';
    inp.placeholder = placeholder;
    inp.addEventListener('input', () => onUpdate(inp.value));
    return inp;
  }

  private ensureCcli(): CcliMetadata {
    if (!this.currentSlide!.ccli_metadata) {
      this.currentSlide!.ccli_metadata = {};
    }
    return this.currentSlide!.ccli_metadata;
  }

  private notifyChange(): void {
    if (this.currentSlide && this.onChangeCb) {
      this.onChangeCb(this.currentSlide);
    }
  }
}
