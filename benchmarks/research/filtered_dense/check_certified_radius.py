"""Check the conservative radius against rational gamma/underflow bounds."""
import argparse
from fractions import Fraction
import hashlib
import json
from pathlib import Path
import struct
import sys


def check():
    unit = Fraction(1, 2 ** 53)
    minimum_f32 = Fraction(1, 2 ** 149)
    minimum_f64 = Fraction(1, 2 ** 1074)
    least_ratio = None
    least_dimension = None
    exact_radius_cases = 0
    maxima = [struct.unpack('!f', struct.pack('!I', exponent << 23 | mantissa))[0]
              for exponent in (0, 1, 126, 127, 253, 254)
              for mantissa in (0, 1, (1 << 23) - 1)]
    for dimension in range(1, 4097):
        def gamma(operations):
            return operations * unit / (1 - operations * unit)
        analytic = (gamma(dimension + 4) + gamma(dimension + 1)) * dimension * minimum_f32
        underflow = 2 * (4 * dimension + 16) * minimum_f64
        radius = 16 * dimension * dimension * unit * minimum_f32
        if radius <= analytic + underflow:
            raise ValueError('radius does not dominate the error allowance')
        ratio = radius / (analytic + underflow)
        if least_ratio is None or ratio < least_ratio:
            least_ratio, least_dimension = ratio, dimension
        for maximum in maxima:
            expected = 16 * dimension * dimension * unit * Fraction.from_float(maximum)
            computed = 8.0 * dimension * dimension * sys.float_info.epsilon * maximum
            if Fraction.from_float(computed) != expected:
                raise ValueError('radius construction rounded unexpectedly')
            exact_radius_cases += 1
    return {'dimensionInequalities': 4096, 'exactRadiusCases': exact_radius_cases,
            'leastMarginDimension': least_dimension,
            'leastRadiusToAllowanceRatio': str(least_ratio),
            'leastRatioApproximation': float(least_ratio),
            'scope': 'Exact rational check of the stated conservative inequality and selected radius products; not a substitute for the rounding-error proof or native tests.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = check()
    result['sourceSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')
    print(json.dumps({key: result[key] for key in ('dimensionInequalities', 'exactRadiusCases', 'leastMarginDimension', 'leastRatioApproximation')}))
