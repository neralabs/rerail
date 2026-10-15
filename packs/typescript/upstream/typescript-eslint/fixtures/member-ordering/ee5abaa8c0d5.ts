
abstract class Foo {
  @Dec public readonly DA: string;
  @Dec protected readonly DB: string;
  @Dec private readonly DC: string;

  public static readonly SA: string;
  public readonly IA: string;
  public abstract readonly AA: string;

  protected static readonly SB: string;
  protected readonly IB: string;
  protected abstract readonly AB: string;

  private static readonly SC: string;
  private readonly IC: string;

  static readonly #SD: string;
  readonly #ID: string;
}
      