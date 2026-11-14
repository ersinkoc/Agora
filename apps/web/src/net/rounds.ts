/** The rounds of a view's reads (one round per refresh): a read that completes after a newer round started is dropped, whether it
 *  succeeded or failed, so an old answer never replaces a current one. */
export class Rounds {
  private n = 0;

  /** Starts a new round: completions bound to an earlier one are dropped from now on. */
  next(): void {
    this.n++;
  }

  /** `f`, run only while no newer round has started since `bind` was called. */
  bind<A extends unknown[]>(f: (...args: A) => void): (...args: A) => void {
    const at = this.n;
    return (...args: A) => {
      if (at === this.n) f(...args);
    };
  }
}
