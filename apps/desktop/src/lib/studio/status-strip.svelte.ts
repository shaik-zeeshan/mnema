// The status strip's one publishable slot: save state.
//
// Direction 02 welds a 24px status strip to the bottom window edge and puts the
// autosave chip in it ("tells you *whether*", per the direction README) while
// the row echo tells you *what*. The strip is rendered by the root layout; the
// save state lives behind the Settings controller context, which only exists
// inside the settings route. Rather than plumb a snippet or a context up
// through the layout, the settings chip publishes its state here and the strip
// reads it.
//
// ponytail: one module-level rune, one shape. If a second surface ever needs to
// push into the strip, give it its own named slot rather than generalising this
// into a registry.

export type StatusSaveTone = "ok" | "busy" | "bad";

export interface StatusSave {
	tone: StatusSaveTone;
	label: string;
}

let save = $state<StatusSave | null>(null);

export const statusSave = {
	get value(): StatusSave | null {
		return save;
	},
	/** Publish (or clear, with `null`) the strip's save state. */
	set(next: StatusSave | null): void {
		save = next;
	},
};
