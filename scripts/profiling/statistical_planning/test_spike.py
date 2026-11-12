import math
import random
import unittest
import spike


class StatisticalPlanningTests(unittest.TestCase):
    def case(self, n=50):
        return spike.Case('fixture', {'a':spike.Source([(i,) for i in range(n)],1),
            'b':spike.Source([(i,7) for i in range(n)],2)},
            [spike.Atom('a',('v0',)),spike.Atom('b',('v0','i:7'))],[],('v0',))

    def test_memory_and_job_work_are_hard_limits(self):
        c=self.case(2000);b=spike.Budget(memory=4096,probes=4,work=7);rng=random.Random(4)
        summaries=spike.collect_summaries(c,rng,b)
        selected,_,_=spike.select(c,summaries,rng,b,1)
        self.assertEqual(selected,(0,1))
        self.assertLessEqual(b.peak,4096);self.assertLessEqual(b.probes,4);self.assertLessEqual(b.work,7)

    def test_a_missed_rare_value_does_not_become_an_empty_relation(self):
        src=spike.Source([(i,i==9999) for i in range(10000)],2)
        a=spike.Atom('r',('v0','i:1'));b=spike.Budget()
        s=spike.Summary(src,[a],random.Random(0),b)
        self.assertEqual(s.counts[a.literals()],1)
        self.assertNotIn(((1,9),),s.counts)

    def test_small_demand_enumeration_has_exact_cost_bounds(self):
        c=self.case(8);p=spike.Probe(c,(0,1),spike.Budget())
        p.extend(32,random.Random(0));lo,hi=p.interval(.01)
        self.assertEqual((lo,hi),(c.truth((0,1))[0],)*2)

    def test_censored_probe_cannot_report_certainty(self):
        c=self.case(20);p=spike.Probe(c,(0,1),spike.Budget(work=8))
        p.extend(32,random.Random(0));self.assertTrue(p.censored)
        self.assertEqual(p.interval(.01),(0,math.inf))

    def test_relation_revision_prevents_reusing_stale_statistics(self):
        c=self.case();rng=random.Random(1);b=spike.Budget()
        summaries=spike.collect_summaries(c,rng,b)
        c.sources['a']=spike.Source([(999,)],1)
        with self.assertRaises(ValueError):spike.select(c,summaries,rng,b,1)

    def test_equal_good_challengers_do_not_block_a_safe_improvement(self):
        cases=spike.synthetic();rng=random.Random(0);b=spike.Budget()
        all_summaries=[spike.collect_summaries(c,rng,b) for c in cases]
        c=cases[0]
        chosen,_,_=spike.select(c,all_summaries[0],rng,b,3)
        self.assertLess(c.truth(chosen)[0]+b.work,c.truth((0,1,2))[0])

    def test_revision_checked_observed_work_and_probe_cache(self):
        c=spike.synthetic()[0];rng=random.Random(5);b=spike.Budget();cache={}
        summary=spike.collect_summaries(c,rng,b)
        revision=tuple((n,s.revision) for n,s in sorted(c.sources.items()))
        evidence=(revision,c.truth((0,1,2))[0])
        first=spike.select(c,summary,rng,b,1,evidence,cache)
        before=b.work
        second=spike.select(c,summary,rng,b,1,evidence,cache)
        self.assertEqual(first[0],second[0]);self.assertEqual(before,b.work)
        with self.assertRaises(ValueError):spike.select(c,summary,rng,b,1,((),100),cache)

    def test_many_rules_share_one_counter_lookup_per_column_mask(self):
        src=spike.Source([(i,i) for i in range(1000)],2)
        atoms=[spike.Atom('r',('v0','i:'+str(i))) for i in range(1000)]
        b=spike.Budget();s=spike.Summary(src,atoms,random.Random(0),b)
        self.assertEqual(s.count_checks,1000)
        self.assertEqual(sum(s.counts.values()),1000)
        self.assertLessEqual(s.actual_bytes(),b.peak)

    def test_joint_sample_preserves_tuple_correlation(self):
        src=spike.Source([(i%20,i%20) for i in range(5000)],2)
        s=spike.Summary(src,[],random.Random(4),spike.Budget())
        self.assertTrue(all(a==b for a,b in s.rows()))

if __name__=='__main__':unittest.main()
