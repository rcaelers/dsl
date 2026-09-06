import io
import json
import unittest
from unittest.mock import patch

from summarize_metal_trace import Table, distribution, main, summarize, summarize_encoding, union_ns


def table(schema, columns, rows):
    return Table(io.StringIO('<trace-query-result><node><schema name="' + schema + '">' +
                            ''.join('<col><mnemonic>' + c + '</mnemonic></col>' for c in columns) +
                            '</schema>' + ''.join('<row>' + r + '</row>' for r in rows) +
                            '</node></trace-query-result>'), schema)


def gpu_row(start, duration, identity=3, process='<process ref="p"/>', depth=0):
    return (f'<start-time>{start}</start-time><duration>{duration}</duration>' + process +
            f'<metal-nesting-level>{depth}</metal-nesting-level><gpu-state>Active</gpu-state>' +
            f'<metal-command-buffer-id>{identity}</metal-command-buffer-id><gpu-channel-name>Vertex</gpu-channel-name>')


def fixtures(extra=()):
    gpu = table('metal-gpu-intervals', ['start', 'duration', 'process', 'event-depth', 'state', 'cmdbuffer-id', 'channel-name'], [
        gpu_row(10, 10, process='<process id="p"><pid id="pid">7</pid></process>'),
        gpu_row(15, 10, process='<process><pid ref="pid"/></process>'),
        gpu_row(12, 2, depth=1),  # Overlap and nested duplicate.
        gpu_row(30, 5, identity=4),  # No encoding label: explicit exclusion.
        gpu_row(1, 10, identity=5), gpu_row(15, 5, identity=5),  # Whole boundary-crossing buffer excluded.
        gpu_row(20, 100, process='<process><pid>99</pid></process>'),
        *extra,
    ])
    app = table('metal-application-intervals', ['process', 'event-type', 'cmdbuffer-id', 'event-label'], [
        '<process><pid>7</pid></process><metal-event-name>Encoding</metal-event-name>' +
        '<metal-command-buffer-id>3</metal-command-buffer-id>' +
        '<formatted-label><metal-object-label id="label">render</metal-object-label><uint64>1</uint64></formatted-label>',
        '<process><pid>99</pid></process><metal-event-name>Encoding</metal-event-name>' +
        '<metal-command-buffer-id>3</metal-command-buffer-id>' +
        '<formatted-label><metal-object-label>unrelated-secret</metal-object-label><uint64>1</uint64></formatted-label>',
    ])
    return gpu, app


def encoding_table(rows):
    return table('metal-application-intervals',
                 ['process', 'event-type', 'cmdbuffer-id', 'event-label', 'thread', 'start', 'duration'], rows)


def encoding_row(start, duration, identity=3, pid=7, tid=11, label='render', nested=False,
                 event='Encoding', thread_pid=None):
    return (f'<process><pid>{pid}</pid></process><metal-event-name>{event}</metal-event-name>'
            f'<metal-command-buffer-id>{identity}</metal-command-buffer-id><formatted-label>'
            f'<metal-object-label>{label}</metal-object-label>' +
            ('' if nested else '<uint64>1</uint64>') + '</formatted-label>'
            f'<thread><tid>{tid}</tid><process><pid>{pid if thread_pid is None else thread_pid}</pid></process></thread>'
            f'<start-time>{start}</start-time><duration>{duration}</duration>')


def gpu_selection(*identities):
    return {'pid': 7, 'window_ns': [10, 100],
            'buffers': [{'command_buffer_id': i, 'label': 'render'} for i in identities]}


class MetalTraceTests(unittest.TestCase):
    def test_process_filter_references_overlap_nested_and_boundary_rows(self):
        report = summarize(*fixtures(), 7, 10, 100)
        self.assertEqual(len(report['buffers']), 1)
        buffer = report['buffers'][0]
        self.assertEqual(buffer['label'], 'render')
        self.assertEqual(buffer['span_ms'], 15 / 1e6)
        self.assertEqual(buffer['active_union_ms'], 15 / 1e6)
        self.assertEqual(len(buffer['intervals']), 2)
        self.assertEqual(report['excluded'], {'non_top_level_active_rows': 1,
                                            'unmatched_buffers': 1, 'outside_or_crossing_window_buffers': 1})
        self.assertNotIn('unrelated-secret', str(report))

    def test_union_excludes_gaps_and_does_not_double_count_overlaps(self):
        self.assertEqual(union_ns([(30, 40), (10, 20), (15, 25), (11, 12)]), 25)
        self.assertEqual(union_ns([]), 0)
        with self.assertRaises(ValueError):
            union_ns([(3, 2)])

    def test_nearest_rank_distributions(self):
        result = distribution([5, 1, 2, 3, 4])
        self.assertEqual(result['p50_ms'], 3)
        self.assertEqual(result['p95_ms'], 5)
        self.assertEqual(result['samples_ms'], [1, 2, 3, 4, 5])
        for values in ([], [float('nan')], [-1]):
            with self.assertRaises(ValueError):
                distribution(values)

    def test_required_render_work_cannot_be_replaced_by_upload_only_work(self):
        summarize(*fixtures(), 7, 10, 100, ['render'])
        with self.assertRaisesRegex(ValueError, 'Missing required GPU'):
            summarize(*fixtures(), 7, 10, 100, ['egui_render'])

    def test_missing_referenced_and_numeric_target_data_fail(self):
        for extra in ([gpu_row(-1, 1)], [gpu_row(10, 1, process='<process ref="missing"/>')],
                      [gpu_row(10, 1, process='<process id="cycle" ref="cycle"/>')],
                      ['<duration>1</duration>']):
            with self.assertRaises(ValueError):
                summarize(*fixtures(extra), 7, 10, 100)
        for pid, start, end in ((0, 0, 1), (7, 2, 1), (7, -1, 1), (8, 10, 100)):
            with self.assertRaises(ValueError):
                summarize(*fixtures(), pid, start, end)

    def test_schema_and_duplicate_identity_validation(self):
        for xml in ('<trace-query-result/>', '<trace-query-result><node/></trace-query-result>',
                    '<trace-query-result><node><schema name="wrong"/></node></trace-query-result>',
                    '<trace-query-result><node><schema name="gpu"/><row><v id="1"/><v id="1"/></row></node></trace-query-result>'):
            with self.assertRaises(ValueError):
                Table(io.StringIO(xml), 'gpu')
        with self.assertRaises(ValueError):
            table('gpu', ['process', 'process'], [])


class EncodingTraceTests(unittest.TestCase):
    def test_cli_encoding_is_opt_in_and_preserves_gpu_report(self):
        reports = []
        for options in ([], ['--include-encoding']):
            gpu, _ = fixtures()
            app = encoding_table([encoding_row(10, 15)])
            output = io.StringIO()
            with patch('sys.argv', ['summarize_metal_trace.py', 'gpu.xml', 'app.xml',
                                    '--pid', '7', '--start-ns', '10', '--end-ns', '100', *options]), \
                    patch('summarize_metal_trace.Table', side_effect=[app, gpu]), \
                    patch('sys.stdout', output):
                main()
            reports.append(json.loads(output.getvalue()))
        encoding = reports[1].pop('encoding')
        self.assertEqual(reports[0], reports[1])
        self.assertEqual(encoding['buffers'][0]['command_buffer_id'], 3)

    def test_identity_thread_and_interval_join_with_boundary_waits(self):
        app = encoding_table([
            encoding_row(20, 40), encoding_row(75, 15, identity=4, tid=12),
            encoding_row(5, 15, identity=5),  # Encoding boundary differs from GPU boundary.
            encoding_row(1, 99, nested=True),  # Ignore nested encoder, even with same identity.
            encoding_row(0, 30, event='Wait for Next Drawable'),
            encoding_row(25, 10, event='Wait for Next Drawable'),  # Overlap is not summed twice.
            encoding_row(50, 70, event='Wait for Next Drawable'),  # Crosses upper window boundary.
            encoding_row(40, 10, tid=12, event='Wait for Next Drawable'),
            encoding_row(20, 40, pid=99, event='Wait for Next Drawable'),
            encoding_row(20, 40, pid=99, label='unrelated-secret'),
            encoding_row(100, 10, event='Wait for Next Drawable'),  # Outside fixed window.
        ])
        report = summarize_encoding(gpu_selection(3, 4, 5), app)
        self.assertEqual(len(report['buffers']), 2)
        first, second = report['buffers']
        self.assertEqual(first['wall_ms'], 40 / 1e6)
        self.assertEqual(first['drawable_wait_overlap_ms'], 25 / 1e6)
        self.assertEqual(first['non_drawable_wait_wall_ms'], 15 / 1e6)
        self.assertEqual(second['drawable_wait_overlap_ms'], 0)
        self.assertEqual(second['non_drawable_wait_wall_ms'], 15 / 1e6)
        self.assertEqual(report['excluded'], {'outside_or_crossing_window_buffers': 1})
        self.assertEqual(len(report['drawable_waits']), 4)
        self.assertEqual(report['summary']['render']['wall']['samples_ms'], [15 / 1e6, 40 / 1e6])
        self.assertNotIn('unrelated-secret', str(report))

    def test_no_drawable_wait_does_not_imply_cpu_time(self):
        report = summarize_encoding(gpu_selection(3), encoding_table([encoding_row(10, 90)]))
        self.assertEqual(report['drawable_waits'], [])
        self.assertEqual(report['buffers'][0]['non_drawable_wait_wall_ms'], 90 / 1e6)
        self.assertIn('not CPU time', report['measurement'])

    def test_duplicate_missing_mismatched_and_boundary_encoding_fail(self):
        for rows, message in [
            ([encoding_row(20, 10), encoding_row(20, 10)], 'Duplicate'),
            ([encoding_row(20, 10, nested=True)], 'Missing'),
            ([encoding_row(20, 10, label='wrong')], 'label'),
            ([encoding_row(1, 10)], 'No matched'),
            ([encoding_row(90, 11)], 'No matched'),
        ]:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                summarize_encoding(gpu_selection(3), encoding_table(rows))

    def test_invalid_thread_and_numeric_values_fail(self):
        for row in [encoding_row(-1, 10), encoding_row(20, -1), encoding_row(20, 10, tid=-1),
                    encoding_row(20, 10, thread_pid=99),
                    encoding_row(20, 10).replace('<tid>11</tid>', '<tid ref="missing"/>')]:
            with self.subTest(row=row), self.assertRaises(ValueError):
                summarize_encoding(gpu_selection(3), encoding_table([row]))
        with self.assertRaisesRegex(ValueError, 'Thread does not belong'):
            summarize_encoding(gpu_selection(3), encoding_table([
                encoding_row(20, 10),
                encoding_row(20, 5, event='Wait for Next Drawable', thread_pid=99),
            ]))


if __name__ == '__main__':
    unittest.main()
