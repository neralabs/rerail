<?php
// A PHPUnit test class (*Test.php): excluded from the rule even though lizard scores it over the limit.

class GradeTest
{
    public function testEveryGrade(): void
    {
        foreach ([95, 85, 75, 65, 55] as $score) {
            if ($score > 90) {
                $this->assertSame('A', grade($score));
            } elseif ($score > 80) {
                $this->assertSame('B', grade($score));
            } elseif ($score > 70) {
                $this->assertSame('C', grade($score));
            } elseif ($score > 60) {
                $this->assertSame('D', grade($score));
            } elseif ($score > 50 || $score === 0) {
                $this->assertSame('F', grade($score));
            }
        }
    }
}
