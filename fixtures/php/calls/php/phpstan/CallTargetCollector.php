<?php
// A PHPStan 2.2.16 collector: for every call PHP-Parser reads (a function call, a method call,
// a nullsafe method call, a static call, `new`), the function or method PHPStan's reflection
// resolves it to. Loaded by phpstan.neon; see generate_php.py.
//
// Each record is [start, end, line, kind, targets, file]:
//   start, end  the call's byte range (startFilePos, endFilePos + 1) in file
//   file        the file the call is written in: a trait's where PHPStan reads a trait's method
//               in the context of a class using it (once per such class, filed under the
//               class's file)
//   kind        function, method, static or new
//   targets     what the callee resolves to, one entry per class PHPStan's type of the receiver
//               names: ["function", name, file] for a user function, ["builtin", name] for one
//               of PHP's, ["method", class, name, file, line] for a user method (the file and
//               line of its declaration, a trait's where a trait declares it), ["builtin-method",
//               class, name] for a method of a builtin class, ["none"] where `new` runs no
//               constructor; ["callable"] for a first-class callable (`f(...)`, no call), and
//               ["unresolved", why] where PHPStan names no function or method (a dynamic callee,
//               a type without the method, an unknown name).

declare(strict_types=1);

namespace RerailOracle;

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Identifier;
use PhpParser\Node\Name;
use PHPStan\Analyser\Scope;
use PHPStan\Collectors\Collector;
use PHPStan\Reflection\ClassReflection;
use PHPStan\Reflection\ExtendedMethodReflection;
use PHPStan\Reflection\ReflectionProvider;

/** @implements Collector<Expr, array{int, int, int, string, list<list<int|string>>, string}> */
final class CallTargetCollector implements Collector
{
    public function __construct(private ReflectionProvider $reflection)
    {
    }

    public function getNodeType(): string
    {
        return Expr::class;
    }

    public function processNode(Node $node, Scope $scope): ?array
    {
        $kind = match (true) {
            $node instanceof Expr\FuncCall => 'function',
            $node instanceof Expr\MethodCall, $node instanceof Expr\NullsafeMethodCall => 'method',
            $node instanceof Expr\StaticCall => 'static',
            $node instanceof Expr\New_ => 'new',
            default => null,
        };
        if ($kind === null) {
            return null;
        }
        try {
            $targets = $this->targets($node, $scope);
        } catch (\Throwable $error) {
            $targets = [['unresolved', 'error: ' . $error->getMessage()]];
        }
        $file = $scope->isInTrait() ? $scope->getTraitReflection()?->getFileName() : null;
        return [$node->getStartFilePos(), $node->getEndFilePos() + 1, $node->getStartLine(), $kind, $targets,
            $file ?? $scope->getFile()];
    }

    /** @return list<list<int|string>> */
    private function targets(Expr $node, Scope $scope): array
    {
        if ($node instanceof Expr\CallLike && !$node instanceof Expr\New_ && $node->isFirstClassCallable()) {
            return [['callable']];
        }
        if ($node instanceof Expr\FuncCall) {
            if (!$node->name instanceof Name) {
                return [['unresolved', 'dynamic function']];
            }
            if (!$this->reflection->hasFunction($node->name, $scope)) {
                return [['unresolved', 'unknown function']];
            }
            $function = $this->reflection->getFunction($node->name, $scope);
            if ($function->isBuiltin()) {
                return [['builtin', $function->getName()]];
            }
            return [['function', $function->getName(), (string) $function->getFileName()]];
        }
        if ($node instanceof Expr\MethodCall || $node instanceof Expr\NullsafeMethodCall) {
            if (!$node->name instanceof Identifier) {
                return [['unresolved', 'dynamic method']];
            }
            return $this->methods($scope->getType($node->var), $node->name->toString(), $scope);
        }
        if ($node instanceof Expr\StaticCall) {
            if (!$node->name instanceof Identifier) {
                return [['unresolved', 'dynamic method']];
            }
            if (!$node->class instanceof Name) {
                return $this->methods($scope->getType($node->class), $node->name->toString(), $scope);
            }
            return $this->methods($scope->resolveTypeByName($node->class), $node->name->toString(), $scope);
        }
        if ($node instanceof Expr\New_) {
            if ($node->class instanceof Node\Stmt\Class_) {
                $class = $this->reflection->getAnonymousClassReflection($node->class, $scope);
                return [$this->constructor($class)];
            }
            if (!$node->class instanceof Name) {
                return [['unresolved', 'dynamic class']];
            }
            $name = $scope->resolveName($node->class);
            if (!$this->reflection->hasClass($name)) {
                return [['unresolved', 'unknown class']];
            }
            return [$this->constructor($this->reflection->getClass($name))];
        }
        return [['unresolved', 'other']];
    }

    /** @return list<list<int|string>> */
    private function methods(\PHPStan\Type\Type $type, string $method, Scope $scope): array
    {
        $classes = $type->getObjectClassReflections();
        if ($classes === []) {
            return [['unresolved', 'no class']];
        }
        $targets = [];
        foreach ($classes as $class) {
            if (!$class->hasMethod($method)) {
                $targets[] = ['unresolved', 'no method'];
                continue;
            }
            // A method only a PHPDoc (`@method`, `@mixin`) or an extension gives: PHP runs
            // `__call` or `__callStatic`.
            if (!$class->isBuiltin() && !$class->getNativeReflection()->hasMethod($method)) {
                $targets[] = ['unresolved', 'magic method'];
                continue;
            }
            $targets[] = $this->method($class->getMethod($method, $scope));
        }
        return $targets;
    }

    /** @return list<int|string> */
    private function constructor(ClassReflection $class): array
    {
        if (!$class->hasConstructor()) {
            return ['none'];
        }
        return $this->method($class->getConstructor());
    }

    /** @return list<int|string> */
    private function method(ExtendedMethodReflection $method): array
    {
        $class = $method->getDeclaringClass();
        if ($class->isBuiltin()) {
            return ['builtin-method', $class->getName(), $method->getName()];
        }
        $native = $class->getNativeReflection();
        if (!$native->hasMethod($method->getName())) {
            // A method only a PHPDoc or an extension declares (`@method`, `__call`).
            return ['unresolved', 'magic method'];
        }
        $declared = $native->getMethod($method->getName());
        // PHP's own: a builtin class's, or one PHP gives every enum (`cases`, `from`, `tryFrom`),
        // which has no line.
        if ($declared->getDeclaringClass()->isInternal() || !$declared->getStartLine()) {
            return ['builtin-method', $declared->getDeclaringClass()->getName(), $declared->getName()];
        }
        return ['method', $declared->getDeclaringClass()->getName(), $declared->getName(),
            (string) $declared->getFileName(), (int) $declared->getStartLine()];
    }
}
