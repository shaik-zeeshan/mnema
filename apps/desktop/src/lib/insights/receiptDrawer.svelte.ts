// receiptDrawer.svelte.ts — one-slot store for the span-scoped receipt drawer
// (Warm Paper redesign, Slice 4). Any surface can call open(activity); the
// shared <InsightsShell> hosts the drawer (<ActivityReceipt>) so receipts open
// over rail + main alike without each surface mounting its own instance.
// ponytail: a singleton slot, no stack — one receipt at a time is the product.
import type { Activity } from "$lib/types/recording";

class ReceiptDrawerStore {
  current = $state<Activity | null>(null);

  open(activity: Activity): void {
    this.current = activity;
  }

  close(): void {
    this.current = null;
  }
}

export const receiptDrawer = new ReceiptDrawerStore();
