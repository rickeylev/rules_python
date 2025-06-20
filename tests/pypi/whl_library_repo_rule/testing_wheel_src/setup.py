from setuptools import setup, find_packages

setup(
    name="testwheel_with_build_files",
    version="0.1.0",
    packages=find_packages(),
    data_files=[('data_files', ['data_files/BUILD'])],
    # Include package data to ensure pkg_with_build_files/* are included
    include_package_data=True,
    # Dummy entry point
    entry_points={
        "console_scripts": [
            "dummy_script=pkg_with_build_files:main",
        ],
    },
)
