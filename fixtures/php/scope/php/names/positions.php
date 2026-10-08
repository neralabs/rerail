<?php
// Every position a class, function or constant name is read in.

namespace Positions;

use Framework\Attributes\Route;
use Framework\Contracts\{Arrayable, Jsonable};
use Framework\Traits\{HasEvents, HasMacros};
use Framework\Base;

#[Route('/'), \Framework\Attributes\Cached]
#[Deprecated]
final class Controller extends Base implements Arrayable, \JsonSerializable, Contracts\Local
{
    use HasEvents, HasMacros {
        HasEvents::boot insteadof HasMacros;
        HasMacros::boot as protected bootMacros;
        register as protected;
    }
    use Concerns\Logging;

    public const int LIMIT = DEFAULT_LIMIT;
    public const ?Base FALLBACK = null;
    #[Constant]
    private const TYPES = [Arrayable::class, Jsonable::class => SORT_ASC];

    #[Inject]
    public ?Arrayable $items = null;
    protected Arrayable&Jsonable $both;
    private (Arrayable&Jsonable)|Base|null $dnf = null;
    public static Base|int $count = 0;

    public function __construct(
        #[Sensitive] private readonly Base $base,
        public Arrayable|string $value = LIMIT,
        Jsonable ...$rest,
    ) {
    }

    #[Route('/show')]
    public function show(int $id, ?Base $base = null, mixed $any = MODE_DEFAULT): ?Jsonable
    {
        try {
            $this->base->find($id)?->render();
            $result = Base::query()->where(COLUMN, $id);
            $closure = function (Arrayable $a) use ($id): Base {
                return new Base();
            };
            $arrow = fn (Jsonable $j): Arrayable => $j;
            $static = static fn (): int => Base::$count + self::LIMIT;
            $anonymous = new #[Anonymous] class ($id) extends Base implements Arrayable {
            };
            if ($result instanceof Jsonable && $result instanceof \Stringable) {
                return $result;
            }
            return match ($id) {
                STATUS_OK, Http\STATUS_CREATED => new Response(),
                default => null,
            };
        } catch (NotFound | \RuntimeException $exception) {
            report($exception);
        } catch (\Throwable) {
            throw new Exceptions\Failed(previous: null);
        }
        switch ($id) {
            case FIRST_CASE:
                break;
        }
        return null;
    }

    public static function make(): static
    {
        return new static();
    }

    public function parent(): self|parent
    {
        return new parent(self::LIMIT, static::class, parent::class);
    }
}

interface Service extends Arrayable, \Countable
{
    public function handle(Request $request): Response;
}

trait Helpers
{
    abstract public function base(): Base;
}

#[Enumerated]
enum Status: string implements Arrayable, HasLabel
{
    use HasEvents;

    #[Label]
    case Active = 'active';
    case Default = DEFAULT_STATUS;

    const ALIAS = self::Active;
}

function top(Base $base = new Base(), int $flags = FLAG_ONE | \FLAG_TWO): never
{
    $list = [KEY => VALUE, 'key' => Base::KEY];
    $value = CONSTANT_A ? CONSTANT_B : constant_c();
    $call = helper(named: ARGUMENT);
    $callable = strlen(...);
    $object = (new Base())->method(ARG);
    $fetch = Base::{$dynamic}();
    $string = "value {$list[INDEX]} $list[KEY_TEXT]";
    $heredoc = <<<EOT
        {$list[HEREDOC_INDEX]} $list[HEREDOC_TEXT]
        EOT;
    $variable = new $className();
    $staticVariable = $className::create();
    $classOf = $object::class;
    $dynamicConstant = Base::{DYNAMIC_NAME};
    $counted = count($list) + \count($list);
    echo INSIDE_ECHO;
    print PRINTED;
    exit(EXIT_CODE);
}

$global = function () {
    static $cache = CACHE_DEFAULT;
    global $config;
    return $cache;
};

abstract class Shape
{
    public function __construct(public Point $origin = new Point(ORIGIN_X, 0))
    {
    }
}
