import sigrokdecode as srd


class Decoder(srd.Decoder):
    api_version = 3
    id = 'test_stacked'
    name = 'Test Stacked'
    longname = 'Test Stacked Decoder'
    desc = 'Checked-in graph-node stacked-decoder test fixture.'
    license = 'mit'
    inputs = ['test_logic']
    outputs = ['test_stacked']
    tags = ['Test']
    channels = ()
    optional_channels = ()
    options = ()
    annotations = ()
    annotation_rows = ()
    binary = ()

    def metadata(self, key, value):
        self.samplerate = value

    def start(self):
        self.output = self.register(srd.OUTPUT_PYTHON, proto_id='test_stacked')
